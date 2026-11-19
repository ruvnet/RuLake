#!/usr/bin/env python3
"""Convert local Parquet shards or newline-delimited JSON exports to a RuLake FsBackend snapshot."""
import argparse
import os
from pathlib import Path
import sqlite3
import struct
import tempfile

import numpy as np
import pyarrow as pa
import pyarrow.compute as pc
import pyarrow.json as paj
import pyarrow.parquet as pq

HEADER = struct.Struct('<8sQII')
MAX_ROWS = 100_000_000
MAX_DIM = 8192
MAX_VECTOR_BYTES = 16 * 1024**3


def batches(path):
    if path.suffix.lower() == '.parquet':
        with pq.ParquetFile(path) as source:
            yield from source.iter_batches(batch_size=1024, columns=['id', 'vector'])
    elif path.suffix.lower() in ('.json', '.jsonl', '.ndjson'):
        schema = pa.schema([('id', pa.int64()), ('vector', pa.list_(pa.float64()))])
        with paj.open_json(path, parse_options=paj.ParseOptions(
            explicit_schema=schema, unexpected_field_behavior='ignore'
        )) as source:
            yield from source
    else:
        raise ValueError(f'{path}: expected .parquet or uncompressed newline-delimited .json, .jsonl or .ndjson')


def records(batch, expected_dim):
    if not {'id', 'vector'}.issubset(batch.schema.names):
        raise ValueError('expected columns named id and vector')
    ids, vectors = batch.column('id'), batch.column('vector')
    if ids.null_count or not pa.types.is_integer(ids.type):
        raise ValueError('id must be a non-null integer')
    ids = ids.to_numpy()
    if ids.min() < 0:
        raise ValueError('id must be nonnegative')
    kind = vectors.type
    if not (pa.types.is_list(kind) or pa.types.is_large_list(kind) or pa.types.is_fixed_size_list(kind)):
        raise ValueError('vector must be a list of floats')
    if vectors.null_count or not (pa.types.is_float32(kind.value_type) or pa.types.is_float64(kind.value_type)):
        raise ValueError('vector must be a non-null list of float32 or float64 values')
    lengths = pc.list_value_length(vectors).to_numpy()
    dim = expected_dim if expected_dim is not None else int(lengths[0])
    if not 1 <= dim <= MAX_DIM or np.any(lengths != dim):
        raise ValueError(f'all vectors must have one width in 1..{MAX_DIM} (expected {dim})')
    values = pc.list_flatten(vectors)
    if values.null_count:
        raise ValueError('null vector component')
    with np.errstate(over='ignore', invalid='ignore'):
        values = values.to_numpy().astype('<f4', copy=False)
    if not np.isfinite(values).all():
        raise ValueError('vector component is outside the finite float32 range')
    block = np.empty(len(ids), dtype=[('id', '<u8'), ('vector', '<f4', (dim,))])
    block['id'] = ids
    block['vector'] = values.reshape(-1, dim)
    return block, dim


def prepare(inputs, output):
    output = Path(output)
    if output.exists():
        raise FileExistsError(f'{output} already exists. Choose a new snapshot filename.')
    count, dim = 0, None
    # Keep uniqueness checks on disk. Only the current Arrow batch and packed records stay in memory.
    with tempfile.TemporaryDirectory(prefix='.rulake-', dir=output.parent) as work:
        pending = Path(work) / 'snapshot'
        ids_db = sqlite3.connect(Path(work) / 'ids.sqlite')
        try:
            ids_db.execute('PRAGMA journal_mode=OFF')
            ids_db.execute('PRAGMA synchronous=OFF')
            ids_db.execute('CREATE TABLE ids (id INTEGER PRIMARY KEY)')
            with pending.open('xb') as stream:
                stream.write(bytes(HEADER.size))
                for path in map(Path, inputs):
                    for batch in batches(path):
                        if not batch.num_rows:
                            continue
                        block, dim = records(batch, dim)
                        count += len(block)
                        if count > MAX_ROWS or count * dim * 4 > MAX_VECTOR_BYTES:
                            raise ValueError('snapshot exceeds RuLake collection limits')
                        try:
                            # SQLite uses signed 64-bit keys. Reinterpreting each uint64 is one-to-one.
                            keys = block['id'].view('<i8').tolist()
                            ids_db.executemany('INSERT INTO ids VALUES (?)', zip(keys))
                        except sqlite3.IntegrityError as exc:
                            raise ValueError(f'{path}: duplicate vector id across snapshot') from exc
                        stream.write(memoryview(block).cast('B'))
                if not count:
                    raise ValueError('snapshot has no vectors')
                stream.seek(0)
                stream.write(HEADER.pack(b'ruvec1\0\0', count, dim, 0))
                stream.flush()
                os.fsync(stream.fileno())
            # The same-filesystem link publishes a complete file and never replaces a concurrent writer.
            os.link(pending, output)
        finally:
            ids_db.close()
    return count, dim


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('inputs', nargs='+', type=Path, help='Files from one completed export, in order')
    parser.add_argument('-o', '--output', required=True, type=Path, help='A new .ruvec1 file')
    args = parser.parse_args()
    try:
        count, dim = prepare(args.inputs, args.output)
    except (OSError, ValueError, sqlite3.Error) as exc:
        parser.exit(1, f'{parser.prog}: {exc}\n')
    print(f'{args.output}: {count} vectors, {dim} dimensions, {args.output.stat().st_size} bytes')
