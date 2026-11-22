#!/usr/bin/env python3
"""Check recorded legal-text provenance and optional downloadable archives.
Only Python's standard library is used. No downloads and no archive extraction.
"""
import argparse
import csv
import hashlib
import io
from pathlib import Path, PurePosixPath
import tarfile

ROOT = Path(__file__).resolve().parents[3]
LEGAL = ROOT / 'docs/third-party-licenses'
COMMON = {
    'LICENSE-MIT': ROOT / 'LICENSE-MIT',
    'LICENSE-APACHE': ROOT / 'LICENSE-APACHE',
    'OFL.txt': ROOT / 'assets/fonts/OFL.txt',
    'LICENSE-FluentSystemIcons.txt': ROOT / 'nice-common/assets/fonts/LICENSE-FluentSystemIcons.txt',
    'NON-AFFILIATION.txt': ROOT / 'docs/NON-AFFILIATION.txt',
    'PLUGIN-INSPIRATION.md': ROOT / 'docs/plugin-inspiration.md',
}


def provenance():
    count = 0
    for index, file_column, url_column, hash_column in [
        (LEGAL / 'upstream.tsv', 2, 3, 4),
        (LEGAL / 'standard/sources.tsv', 0, 1, 2),
    ]:
        for row in csv.reader((s for s in index.read_text().splitlines() if s and not s.startswith('#')), delimiter='\t'):
            path = PurePosixPath(row[file_column])
            assert not path.is_absolute() and '..' not in path.parts, f'unsafe provenance path: {path}'
            text = index.parent / path
            assert row[url_column].startswith('https://'), f'missing upstream origin: {text}'
            assert hashlib.sha256(text.read_bytes()).hexdigest() == row[hash_column], f'changed legal text: {text}'
            count += 1
    print(f'Checked {count} recorded license-text origins/checksums')


def archive(path):
    with tarfile.open(path, 'r:gz') as package:
        members = package.getmembers()
        roots = set()
        for entry in members:
            name = PurePosixPath(entry.name)
            assert not name.is_absolute() and '..' not in name.parts and '\\' not in entry.name, f'unsafe path: {entry.name}'
            assert not entry.issym() and not entry.islnk(), f'unreviewed archive link: {entry.name}'
            assert entry.isfile() or entry.isdir(), f'special file: {entry.name}'
            assert name.name != '.DS_Store' and not name.name.startswith('._'), f'OS droppings: {entry.name}'
            assert '-settings.' not in name.name, f'private configuration: {entry.name}'
            assert name.suffix.lower() not in {'.nam', '.wav', '.ron', '.png', '.jpg', '.jpeg', '.svg', '.pdf', '.rs', '.c', '.h'}, f'unreviewed asset/source: {entry.name}'
            roots.add(name.parts[0])
        assert len(roots) == 1, 'expected one platform release directory'
        root = roots.pop()

        def read(name):
            stream = package.extractfile(root + '/' + name)
            assert stream is not None, f'missing release text: {name}'
            return stream.read()

        for name, source in COMMON.items():
            assert read(name) == source.read_bytes(), f'wrong release text: {name}'
        notices = read('THIRD-PARTY-NOTICES.txt').decode()
        sources = read('MPL-SOURCES.txt').decode()
        rows = list(csv.DictReader(io.StringIO(read('DEPENDENCIES.tsv').decode()), delimiter='\t'))
        assert rows, 'empty dependency inventory'
        for row in rows:
            assert row['package'] + ' ' + row['version'] in notices, 'missing dependency attribution'
            if 'MPL-2.0' in row['selected terms']:
                assert row['source'] in sources and row['version'] in sources, 'missing MPL source location'
        if any(row['package'] == 'NeuralAmpModeler-rs' for row in rows):
            assert read('NOTICE-NeuralAmpModeler-rs.txt') == (ROOT / 'nice-nam/NOTICE.txt').read_bytes()
        # The large collection is shared, never repeated inside each plugin.
        assert sum(PurePosixPath(m.name).name == 'THIRD-PARTY-NOTICES.txt' for m in members) == 1
        print(f'Checked {path}: {len(rows)} dependency entries, one shared notice collection')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('archives', type=Path, nargs='*')
    args = parser.parse_args()
    provenance()
    for item in args.archives:
        archive(item)
