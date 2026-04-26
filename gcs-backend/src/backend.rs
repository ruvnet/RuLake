//! `GcsParquetBackend` — `BackendAdapter` over Parquet files on GCS.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use arrow::array::{Array, AsArray, Float32Array, Int64Array, ListArray};
use arrow::datatypes::Float32Type;
use futures::StreamExt;
use object_store::path::Path as OsPath;
use object_store::{ObjectMeta, ObjectStore, ObjectStoreExt, gcp::GoogleCloudStorageBuilder};
use parquet::arrow::ParquetRecordBatchStreamBuilder;
use parquet::arrow::async_reader::ParquetObjectReader;

use ruvector_rulake::backend::{
    BackendAdapter, CollectionId, PulledBatch, MAX_PULLED_DIM,
};
use ruvector_rulake::bundle::{Generation, RuLakeBundle};
use ruvector_rulake::error::{Result, RuLakeError};

// ─── Config ────────────────────────────────────────────────────────────

/// One registered (collection name → object on GCS) mapping.
#[derive(Debug, Clone)]
pub struct GcsParquetCollection {
    /// Collection name as ruLake sees it.
    pub name: String,
    /// Object path inside the bucket — e.g. `embeddings/2026-04/docs.parquet`.
    pub object: String,
    /// Operator-supplied dim. Optional — we read it from the Parquet
    /// schema on first pull if absent. Caching it here lets the
    /// `current_bundle` fast path avoid a footer read.
    pub dim: Option<usize>,
}

// ─── Backend ───────────────────────────────────────────────────────────

/// Parquet-on-GCS `BackendAdapter`. Cheap to clone (Arc-wrapped state).
#[derive(Clone)]
pub struct GcsParquetBackend {
    id: String,
    bucket: String,
    inner: Arc<Inner>,
}

struct Inner {
    store: Arc<dyn ObjectStore>,
    collections: RwLock<HashMap<CollectionId, GcsParquetCollection>>,
    /// Cached `(dim, generation)` per collection. Populated on first
    /// pull; refreshed lazily when `current_bundle` finds a generation
    /// bump on GCS.
    schema_cache: RwLock<HashMap<CollectionId, SchemaCache>>,
    /// Single-threaded current-thread runtime that hosts the async
    /// `object_store` / `parquet` calls. Owned by the backend so the
    /// sync `BackendAdapter` trait stays satisfied without forcing
    /// every caller onto tokio.
    runtime: Arc<tokio::runtime::Runtime>,
}

#[derive(Debug, Clone)]
struct SchemaCache {
    dim: usize,
    generation: u64,
}

impl GcsParquetBackend {
    /// Internal constructor that takes an explicit id and store —
    /// exists so tests can hand us an in-memory ObjectStore without
    /// touching the network.
    pub fn with_store(
        id: impl Into<String>,
        bucket: impl Into<String>,
        store: Arc<dyn ObjectStore>,
    ) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| RuLakeError::InvalidParameter(format!("tokio rt: {e}")))?;
        Ok(Self {
            id: id.into(),
            bucket: bucket.into(),
            inner: Arc::new(Inner {
                store,
                collections: RwLock::new(HashMap::new()),
                schema_cache: RwLock::new(HashMap::new()),
                runtime: Arc::new(runtime),
            }),
        })
    }

    /// Build a backend pointing at a real GCS bucket. ADC auth.
    pub fn open_gcs(id: impl Into<String>, bucket: impl Into<String>) -> Result<Self> {
        let id = id.into();
        let bucket = bucket.into();
        let store = GoogleCloudStorageBuilder::new()
            .with_bucket_name(&bucket)
            .build()
            .map_err(|e| RuLakeError::Backend {
                backend: id.clone(),
                detail: format!("GCS init: {e}"),
            })?;
        Self::with_store(id, bucket, Arc::new(store))
    }

    /// Register `collection_name` → `gs://<bucket>/<object>`. The
    /// object doesn't have to exist yet; the first pull will fail
    /// with `RuLakeError::Backend` if it's missing.
    pub fn register(&self, c: GcsParquetCollection) {
        self.inner
            .collections
            .write()
            .unwrap()
            .insert(c.name.clone(), c);
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    fn collection(&self, name: &str) -> Result<GcsParquetCollection> {
        self.inner
            .collections
            .read()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| RuLakeError::UnknownCollection {
                backend: self.id.clone(),
                collection: name.to_string(),
            })
    }

    fn cached_schema(&self, name: &str) -> Option<SchemaCache> {
        self.inner.schema_cache.read().unwrap().get(name).cloned()
    }

    fn put_cached_schema(&self, name: &str, sc: SchemaCache) {
        self.inner
            .schema_cache
            .write()
            .unwrap()
            .insert(name.to_string(), sc);
    }
}

// ─── BackendAdapter impl ──────────────────────────────────────────────

impl BackendAdapter for GcsParquetBackend {
    fn id(&self) -> &str {
        &self.id
    }

    fn list_collections(&self) -> Result<Vec<CollectionId>> {
        Ok(self
            .inner
            .collections
            .read()
            .unwrap()
            .keys()
            .cloned()
            .collect())
    }

    fn pull_vectors(&self, collection: &str) -> Result<PulledBatch> {
        let c = self.collection(collection)?;
        let path = OsPath::from(c.object.clone());
        let store = Arc::clone(&self.inner.store);

        let id = self.id.clone();
        let id_for_err = id.clone();
        let (ids, vectors, dim, generation) = self
            .inner
            .runtime
            .block_on(async move {
                let head = store.head(&path).await.map_err(|e| RuLakeError::Backend {
                    backend: id_for_err.clone(),
                    detail: format!("HEAD {path}: {e}"),
                })?;
                let generation = generation_of(&head);
                let reader = ParquetObjectReader::new(store, path.clone());
                let builder = ParquetRecordBatchStreamBuilder::new(reader)
                    .await
                    .map_err(|e| RuLakeError::Backend {
                        backend: id_for_err.clone(),
                        detail: format!("parquet open {path}: {e}"),
                    })?;
                let stream = builder.build().map_err(|e| RuLakeError::Backend {
                    backend: id_for_err.clone(),
                    detail: format!("parquet build {path}: {e}"),
                })?;
                read_batches(stream, &id_for_err, &path).await
                    .map(|(ids, vectors, dim)| (ids, vectors, dim, generation))
            })?;

        // Cache the (dim, generation) for cheap current_bundle() later.
        self.put_cached_schema(
            collection,
            SchemaCache {
                dim,
                generation,
            },
        );

        if dim > MAX_PULLED_DIM {
            return Err(RuLakeError::InvalidParameter(format!(
                "GcsParquetBackend: dim={dim} > MAX_PULLED_DIM={MAX_PULLED_DIM}"
            )));
        }

        Ok(PulledBatch {
            collection: collection.to_string(),
            ids,
            vectors,
            dim,
            generation,
        })
    }

    fn generation(&self, collection: &str) -> Result<u64> {
        let c = self.collection(collection)?;
        let path = OsPath::from(c.object.clone());
        let store = Arc::clone(&self.inner.store);
        let id = self.id.clone();
        self.inner.runtime.block_on(async move {
            let head = store.head(&path).await.map_err(|e| RuLakeError::Backend {
                backend: id.clone(),
                detail: format!("HEAD {path}: {e}"),
            })?;
            Ok(generation_of(&head))
        })
    }

    /// Cheap-path override — the ADR-004 §Resources contract requires
    /// every backend used behind `rulake://bundle/{backend}/{collection}`
    /// to *not* call the default impl (which does a full `pull_vectors`).
    /// We do a HEAD on the object plus a one-time Parquet-footer read
    /// to learn the dim, then synthesize the bundle.
    fn current_bundle(
        &self,
        collection: &str,
        rotation_seed: u64,
        rerank_factor: usize,
    ) -> Result<RuLakeBundle> {
        let c = self.collection(collection)?;
        let path = OsPath::from(c.object.clone());
        let store = Arc::clone(&self.inner.store);
        let id = self.id.clone();
        let bucket = self.bucket.clone();

        // Fetch generation cheaply.
        let id_head = id.clone();
        let head_path = path.clone();
        let head_store = Arc::clone(&store);
        let generation = self.inner.runtime.block_on(async move {
            let head = head_store.head(&head_path).await.map_err(|e| RuLakeError::Backend {
                backend: id_head.clone(),
                detail: format!("HEAD {head_path}: {e}"),
            })?;
            Ok::<u64, RuLakeError>(generation_of(&head))
        })?;

        // Resolve dim — cached if we've seen this collection before, or
        // operator-supplied at register time, or read the Parquet footer.
        let dim = if let Some(cached) = self.cached_schema(collection) {
            if cached.generation == generation {
                cached.dim
            } else {
                // Generation changed — schema *might* have too. Re-read footer.
                self.read_dim_via_footer(&path, &id)?
            }
        } else if let Some(d) = c.dim {
            d
        } else {
            self.read_dim_via_footer(&path, &id)?
        };

        // Update cache.
        self.put_cached_schema(
            collection,
            SchemaCache {
                dim,
                generation,
            },
        );

        let data_ref = format!("gs://{bucket}/{}", c.object);
        Ok(RuLakeBundle::new(
            data_ref,
            dim,
            rotation_seed,
            rerank_factor,
            Generation::Num(generation),
        ))
    }
}

impl GcsParquetBackend {
    fn read_dim_via_footer(&self, path: &OsPath, id: &str) -> Result<usize> {
        let store = Arc::clone(&self.inner.store);
        let id = id.to_string();
        let path = path.clone();
        self.inner.runtime.block_on(async move {
            let reader = ParquetObjectReader::new(store, path.clone());
            let builder = ParquetRecordBatchStreamBuilder::new(reader)
                .await
                .map_err(|e| RuLakeError::Backend {
                    backend: id.clone(),
                    detail: format!("parquet footer {path}: {e}"),
                })?;
            let schema = builder.schema();
            // Find the `vector` column and extract its list element width.
            // For a fixed-length list this is on `Field::data_type()`.
            let field = schema
                .field_with_name("vector")
                .map_err(|e| RuLakeError::Backend {
                    backend: id.clone(),
                    detail: format!("schema: no `vector` column ({e})"),
                })?;
            list_element_count_estimate(field.data_type())
                .ok_or_else(|| RuLakeError::Backend {
                    backend: id.clone(),
                    detail: format!(
                        "schema: `vector` column is not a fixed-size list of floats (type: {:?})",
                        field.data_type()
                    ),
                })
        })
    }
}

// ─── Helpers ───────────────────────────────────────────────────────────

fn generation_of(meta: &ObjectMeta) -> u64 {
    // GCS exposes the per-object generation number in the e_tag /
    // version header; object_store surfaces it in `version` (when
    // available) or falls back to `last_modified` epoch seconds.
    if let Some(version) = &meta.version {
        if let Ok(n) = version.parse::<u64>() {
            return n;
        }
    }
    meta.last_modified.timestamp() as u64
}

/// Returns `Some(dim)` if `dt` is a `LIST<FLOAT32>` whose offsets we
/// can read at footer time. For `FixedSizeList<FLOAT32, N>` we get N
/// directly; for variable `List<FLOAT32>` we return `None` (caller
/// falls back to reading the first row).
fn list_element_count_estimate(dt: &arrow::datatypes::DataType) -> Option<usize> {
    use arrow::datatypes::DataType;
    match dt {
        DataType::FixedSizeList(_, n) => Some(*n as usize),
        DataType::List(_) | DataType::LargeList(_) => None,
        _ => None,
    }
}

/// Read every batch from the Parquet stream, decoding `id: INT64` and
/// `vector: LIST<FLOAT>` columns into the `PulledBatch` shape.
async fn read_batches(
    mut stream: parquet::arrow::async_reader::ParquetRecordBatchStream<ParquetObjectReader>,
    id: &str,
    path: &OsPath,
) -> Result<(Vec<u64>, Vec<Vec<f32>>, usize)> {
    let mut all_ids: Vec<u64> = Vec::new();
    let mut all_vectors: Vec<Vec<f32>> = Vec::new();
    let mut dim: Option<usize> = None;

    while let Some(batch) = stream.next().await {
        let batch = batch.map_err(|e| RuLakeError::Backend {
            backend: id.to_string(),
            detail: format!("parquet batch {path}: {e}"),
        })?;
        let ids_col = batch
            .column_by_name("id")
            .ok_or_else(|| RuLakeError::Backend {
                backend: id.to_string(),
                detail: "schema: missing `id` column".into(),
            })?
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or_else(|| RuLakeError::Backend {
                backend: id.to_string(),
                detail: "schema: `id` column is not INT64".into(),
            })?;
        let vec_col_any = batch
            .column_by_name("vector")
            .ok_or_else(|| RuLakeError::Backend {
                backend: id.to_string(),
                detail: "schema: missing `vector` column".into(),
            })?;

        // Two cases: FixedSizeList<Float32, N> or List<Float32>.
        let n_rows = batch.num_rows();
        for row in 0..n_rows {
            let row_vec = extract_vector_row(vec_col_any.as_ref(), row, id, path)?;
            if let Some(d) = dim {
                if row_vec.len() != d {
                    return Err(RuLakeError::DimensionMismatch {
                        expected: d,
                        actual: row_vec.len(),
                    });
                }
            } else {
                dim = Some(row_vec.len());
            }
            all_vectors.push(row_vec);
            // INT64 → u64 by bit pattern.
            all_ids.push(ids_col.value(row) as u64);
        }
    }

    let dim = dim.ok_or_else(|| RuLakeError::Backend {
        backend: id.to_string(),
        detail: "parquet: no rows".into(),
    })?;
    Ok((all_ids, all_vectors, dim))
}

fn extract_vector_row(
    arr: &dyn Array,
    row: usize,
    backend: &str,
    path: &OsPath,
) -> Result<Vec<f32>> {
    use arrow::datatypes::DataType;
    match arr.data_type() {
        DataType::FixedSizeList(field, _n) => {
            if !matches!(field.data_type(), DataType::Float32) {
                return Err(RuLakeError::Backend {
                    backend: backend.to_string(),
                    detail: format!(
                        "schema: vector element type {:?} (expected Float32) at {path}",
                        field.data_type()
                    ),
                });
            }
            let list = arr.as_fixed_size_list();
            let values = list.value(row);
            let f32s = values.as_any().downcast_ref::<Float32Array>().ok_or_else(|| {
                RuLakeError::Backend {
                    backend: backend.to_string(),
                    detail: format!("vector value not a Float32Array at row {row}"),
                }
            })?;
            Ok(f32s.values().to_vec())
        }
        DataType::List(field) => {
            if !matches!(field.data_type(), DataType::Float32) {
                return Err(RuLakeError::Backend {
                    backend: backend.to_string(),
                    detail: format!(
                        "schema: vector element type {:?} (expected Float32) at {path}",
                        field.data_type()
                    ),
                });
            }
            let list = arr.as_any().downcast_ref::<ListArray>().ok_or_else(|| {
                RuLakeError::Backend {
                    backend: backend.to_string(),
                    detail: "vector column is not a ListArray".into(),
                }
            })?;
            let values = list.value(row);
            let f32s = values.as_primitive::<Float32Type>();
            Ok(f32s.values().to_vec())
        }
        other => Err(RuLakeError::Backend {
            backend: backend.to_string(),
            detail: format!("vector column type unsupported: {other:?} (need List<Float32> or FixedSizeList<Float32, N>) at {path}"),
        }),
    }
}
