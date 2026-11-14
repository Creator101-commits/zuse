#!/usr/bin/env python3
"""Add distribution assets without replacing existing versioned downloads."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile

from distribution import REPOSITORY, release_version

CHECKSUM_FILES = {"SHA256SUMS", "SHA256SUMS.cli"}


def gh(*arguments):
    return subprocess.check_output(['gh', 'release', *arguments, '-R', REPOSITORY], text=True)


def missing_assets(tag, names):
    """Every release ships every format; name what is absent from `names`."""
    version = release_version(tag)
    suffixes = {
        'linux-x86_64': ('.tar.gz', '.deb', '.rpm'),
        'linux-aarch64': ('.tar.gz', '.deb', '.rpm'),
        'macos-aarch64': ('.tar.gz', '.dmg'),
        'macos-x86_64': ('.tar.gz', '.dmg'),
        'windows-x86_64': ('.zip', '-setup.exe'),
    }
    required = {'install.sh'} | {f'zuse-{version}-{target}{suffix}' for target, kinds in suffixes.items() for suffix in kinds}
    return sorted(required - set(names))


def require_complete(tag, names):
    missing = missing_assets(tag, names)
    if missing:
        raise ValueError('Release is missing: ' + ', '.join(missing))


def write_release_checksums(output, checksums=None):
    """Older app updaters interpret every zuse-* name as an app version.
    Keep CLI hashes separate while verifying every published download."""
    if checksums is None:
        checksums = {path.name: hashlib.sha256(path.read_bytes()).hexdigest()
                     for path in output.iterdir()
                     if path.is_file() and (path.name.startswith('zuse-') or path.name == 'install.sh')}
    files = []
    cli = {name: digest for name, digest in checksums.items() if name.startswith('zuse-cli-')}
    if cli:
        path = output / 'SHA256SUMS.cli'
        path.write_text(''.join(f'{digest}  {name}\n' for name, digest in sorted(cli.items())))
        files.append(path)
    path = output / 'SHA256SUMS'
    path.write_text(''.join(f'{digest}  {name}\n' for name, digest in sorted(checksums.items()) if name not in cli))
    files.append(path)
    return files


def publish(tag, output, command=gh, complete=False):
    release_version(tag)
    output = output.resolve()
    assets = {asset['name'] for asset in json.loads(command('view', tag, '--json', 'assets'))['assets']}
    if complete:
        require_complete(tag, assets | {path.name for path in output.iterdir()})
    checksums = {}
    for checksum_file in sorted(CHECKSUM_FILES & assets):
        command('download', tag, '--pattern', checksum_file, '--dir', str(output), '--clobber')
        seen = set()
        for line in (output / checksum_file).read_text().splitlines():
            digest, name = line.split(maxsplit=1)
            name = name.lstrip('*')
            if name in seen or not re.fullmatch('[0-9a-f]{64}', digest):
                raise ValueError('Invalid published checksum list')
            if name in checksums and checksums[name] != digest:
                raise ValueError('Conflicting published checksum lists')
            seen.add(name)
            checksums[name] = digest
    if set(checksums) - assets:
        raise ValueError('Published checksums reference missing assets')
    # Recover a previous interrupted upload, before its checksum update landed.
    with tempfile.TemporaryDirectory(prefix='zuse-published-') as temporary:
        for name in sorted(assets - CHECKSUM_FILES - set(checksums)):
            if name != 'install.sh' and not re.fullmatch(r'zuse-[0-9A-Za-z._-]+', name):
                raise ValueError('Unexpected published asset name')
            command('download', tag, '--pattern', name, '--dir', temporary)
            checksums[name] = hashlib.sha256((Path(temporary) / name).read_bytes()).hexdigest()
    # Upload new versioned files first. Retries retain previously published bytes,
    # including signatures and compression timestamps from an earlier attempt.
    for path in sorted(output.glob('zuse-*')):
        if path.name in assets:
            print('Keeping published asset:', path.name)
            continue
        command('upload', tag, str(path))
        checksums[path.name] = hashlib.sha256(path.read_bytes()).hexdigest()
    installer = output / 'install.sh'
    digest = hashlib.sha256(installer.read_bytes()).hexdigest()
    command('upload', tag, str(installer), '--clobber')
    checksums[installer.name] = digest
    for sums in write_release_checksums(output, checksums):
        command('upload', tag, str(sums), '--clobber')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('tag')
    parser.add_argument('output', type=Path)
    parser.add_argument('--check-only', action='store_true', help='Verify the directory holds every format and publish nothing')
    parser.add_argument('--write-checksums', action='store_true', help='Write app and CLI checksum lists without publishing')
    args = parser.parse_args()
    if args.write_checksums:
        release_version(args.tag)
        write_release_checksums(args.output)
    elif args.check_only:
        require_complete(args.tag, [path.name for path in args.output.iterdir()])
    else:
        publish(args.tag, args.output, complete=True)
