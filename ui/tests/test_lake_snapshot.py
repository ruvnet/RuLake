"""Run with python ui/tests/test_lake_snapshot.py after installing pyarrow and numpy."""
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import struct
import sys
import tempfile
import unittest

import pyarrow as pa
import pyarrow.parquet as pq

ASSETS = Path(__file__).resolve().parents[1] / 'public' / 'lakes'
sys.dont_write_bytecode = True
sys.path.insert(0, str(ASSETS))
from prepare_snapshot import prepare


class SnapshotTests(unittest.TestCase):
    def setUp(self):
        work = tempfile.TemporaryDirectory()
        self.addCleanup(work.cleanup)
        self.root = Path(work.name)
        self.output = self.root / 'out.ruvec1'

    def shard(self, name, ids, vectors, kind=pa.list_(pa.float64())):
        path = self.root / name
        pq.write_table(pa.table({'id': ids, 'vector': pa.array(vectors, kind),
                                 'ignored': ['extra'] * len(ids)}), path)
        return path

    def test_sample_matches_download(self):
        self.assertEqual(prepare([ASSETS / 'sample.parquet'], self.output), (8, 4))
        self.assertEqual(self.output.read_bytes(), (ASSETS / 'sample.ruvec1').read_bytes())

    def test_shards_and_full_width_ids(self):
        exact = 2**64 - 1
        a = self.shard('one.parquet', pa.array([exact], pa.uint64()), [[1, 2]], pa.list_(pa.float32(), 2))
        b = self.root / 'two.json'
        b.write_text(json.dumps({'id': 2**53 + 1, 'vector': [3., 4.]}) + '\n')
        self.assertEqual(prepare([a, b], self.output), (2, 2))
        data = self.output.read_bytes()
        self.assertEqual(struct.unpack_from('<8sQII', data), (b'ruvec1\0\0', 2, 2, 0))
        self.assertEqual(list(struct.iter_unpack('<Qff', data[24:])), [(exact, 1., 2.), (2**53 + 1, 3., 4.)])

    def test_repeat_id_across_files(self):
        a = self.shard('first.parquet', [17], [[1]])
        b = self.shard('second.parquet', [17], [[2]])
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            prepare([a, b], self.output)
        self.assertFalse(self.output.exists())

    def test_invalid_rows_never_publish(self):
        for ids, vectors in [([1, 2], [[1], [1, 2]]), ([-2], [[1]]), ([None], [[1]]),
                             ([1], [None]), ([1], [[None]]), ([1], [[]]), ([1], [[1e100]]),
                             ([1], [[float('nan')]]), ([1], [[float('inf')]])]:
            with self.subTest(ids=ids, vectors=vectors), self.assertRaises(ValueError):
                prepare([self.shard('bad.parquet', ids, vectors)], self.output)
            self.assertFalse(self.output.exists())

    def test_invalid_schema_and_empty(self):
        bad = self.root / 'bad.parquet'
        for table in [pa.table({'id': [1], 'value': [2]}), pa.table({'id': [1], 'vector': ['hello']}),
                      pa.table({'id': pa.array([], pa.int64()), 'vector': pa.array([], pa.list_(pa.float32()))})]:
            pq.write_table(table, bad)
            with self.assertRaises(ValueError):
                prepare([bad], self.output)
            self.assertFalse(self.output.exists())

    def test_two_writers_publish_once(self):
        def write(_):
            try:
                return prepare([ASSETS / 'sample.parquet'], self.output)
            except FileExistsError:
                return None

        with ThreadPoolExecutor(2) as pool:
            self.assertCountEqual(pool.map(write, range(2)), [(8, 4), None])
        self.assertEqual(self.output.read_bytes(), (ASSETS / 'sample.ruvec1').read_bytes())


if __name__ == '__main__':
    unittest.main()
