# Exercised by guest_start/tests.rs: the in-computer start's waiting,
# acceptance and credential handling, without root, systemd or a provider.
import base64, hashlib, json, os, stat, subprocess, sys, tempfile, threading, time
from pathlib import Path

ns = {'__name__': 'fresh_start_under_test'}
exec(compile(Path(sys.argv[1]).read_text(), 'managed-base-fresh-start.py', 'exec'), ns)
tmp = Path(tempfile.mkdtemp())


def check(condition, message):
    if not condition:
        raise SystemExit('FAILED: ' + message)


# The loader's path rule, for a chosen owner.
uid = os.getuid()
safe = tmp / 'a' / 'b'
safe.mkdir(parents=True)
check(not ns['path_safe'](safe), 'root-owned rule must refuse a non-root tree')
os.chmod(tmp / 'a', 0o775)
check(not ns['path_safe'](safe, owner=uid), 'group-writable ancestor must be unsafe')
os.chmod(tmp / 'a', 0o755)
link = tmp / 'link'
link.symlink_to(safe)
check(not ns['path_safe'](link, owner=uid), 'symlink must be unsafe')
check(not ns['path_safe'](tmp / 'missing', owner=uid), 'missing path is not yet safe')

# An attribute event wakes the wait long before the fallback interval.
watched = tmp / 'watched'
watched.mkdir()
os.chmod(watched, 0o775)
watch = ns['Watch']([watched])
threading.Timer(0.3, lambda: os.chmod(watched, 0o755)).start()
started = time.monotonic()
ok = ns['wait_until'](lambda: not (watched.stat().st_mode & 0o022), time.monotonic() + 10, watch, fallback=5)
took = time.monotonic() - started
check(ok and 0.25 <= took < 1.5, f'event wake took {took:.2f}s')

# A missed event (nothing watched changes) still ends within the fallback.
flip_at = time.monotonic() + 0.6
started = time.monotonic()
ok = ns['wait_until'](lambda: time.monotonic() >= flip_at, time.monotonic() + 10, watch, fallback=0.25)
took = time.monotonic() - started
check(ok and 0.6 <= took < 0.95, f'fallback wake took {took:.2f}s')

# The hard deadline ends the wait with False.
started = time.monotonic()
ok = ns['wait_until'](lambda: False, time.monotonic() + 0.5, watch, fallback=0.25)
took = time.monotonic() - started
check(not ok and 0.5 <= took < 0.9, f'deadline took {took:.2f}s')
watch.close()

# Receipt acceptance mirrors the server.
pins = {k: c * 64 for k, c in (('runtimeSha256', 'a'), ('supervisorSha256', 'b'), ('serviceSha256', 'c'), ('manifestSha256', 'd'), ('helperSha256', 'e'))}
operation = 'op-' + os.urandom(6).hex()
request = {'version': 1, 'provider': 'boat', 'purpose': 'initial-allocation', 'operationId': operation,
           'workspaceId': 'workspace-one', 'generation': 1, 'storageIncarnationId': 'disk-one',
           'sourceMachineId': 'machine-one', 'imagePins': pins}
ready = {**request, 'status': 'ready', 'requestSha256': hashlib.sha256(ns['canonical'](request)).hexdigest(),
         'unitStartSubmitted': True, 'resident': {'ipcVerified': True, 'unbound': True, 'childAlive': False,
         'recoveryAlive': False, 'residentPid': 42, 'residentStartTicks': 7, 'bootNonceSha256': 'f' * 64}}
check(ns['accepted_receipt'](json.dumps(ready), request) is not None, 'valid receipt accepted')
for change in ({'childAlive': True}, {'unbound': False}, {'residentPid': 1}, {'bootNonceSha256': 'x'}):
    bad = json.loads(json.dumps(ready)); bad['resident'].update(change)
    check(ns['accepted_receipt'](json.dumps(bad), request) is None, f'receipt with {change} refused')
check(ns['accepted_receipt'](json.dumps({**ready, 'workspaceId': 'other'}), request) is None, 'binding mismatch refused')
check(ns['accepted_receipt'](json.dumps({**ready, 'requestSha256': '0' * 64}), request) is None, 'digest mismatch refused')


class Result:
    def __init__(self, returncode, stdout):
        self.returncode, self.stdout = returncode, stdout.encode() if isinstance(stdout, str) else stdout


unsafe = Result(1, json.dumps({'version': 1, 'status': 'rejected', 'code': 'fresh_boot_loader_path_unsafe'}))
check(ns['helper_state'](unsafe) == 'retry', 'loader path refusal retries')
check(ns['helper_state'](Result(0, json.dumps({**request, 'status': 'starting'}))) == 'retry', 'starting retries')
check(ns['helper_state'](Result(1, json.dumps({'status': 'rejected', 'code': 'fresh_boot_file_hash_mismatch'}))) == 'rejected', 'hash mismatch rejected')
check(ns['helper_state'](Result(0, json.dumps(ready))) == 'ready', 'ready')

# Dispatch: the credential leaves /tmp at once into the private directory,
# and a repeated dispatch never starts twice.
class Units:
    calls = []
    @staticmethod
    def run(argv, **_):
        Units.calls.append(argv)
        return Result(0, '')
ns['subprocess'] = type('S', (), {'run': staticmethod(Units.run)})
ns['private_root'] = lambda root: root.mkdir(mode=0o700, exist_ok=True)
root = tmp / 'run'
staged = Path('/tmp/zuse-rust-boot-' + operation)
args = {'version': 1, 'operationId': operation, 'staged': str(staged), 'starter': '# starter', 'loader': '# loader',
        'helperSha256': 'e' * 64, 'request': base64.b64encode(json.dumps(request).encode()).decode(),
        'launch': 'true', 'deadlineSeconds': 60}
staged.write_bytes(b'private-boot-credential')
check(ns['dispatch'](args, root) == {'status': 'accepted'}, 'first dispatch accepted')
directory = root / operation
check(not staged.exists(), 'staged credential removed immediately')
check((directory / 'boot').read_bytes() == b'private-boot-credential', 'credential kept privately')
check(stat.S_IMODE((directory / 'boot').stat().st_mode) == 0o600, 'credential file is 0600')
check(json.loads((directory / 'status.json').read_bytes())['status'] == 'waiting', 'status waiting')
check(len(Units.calls) == 1 and Units.calls[0][0] == 'systemd-run', 'one waiting unit started')
staged.write_bytes(b'repeated-upload')
check(ns['dispatch'](args, root) == {'status': 'accepted', 'existing': 'waiting'}, 'repeat is accepted')
check(not staged.exists() and len(Units.calls) == 1, 'repeat starts nothing and drops its upload')
try:
    ns['dispatch']({**args, 'staged': '/tmp/elsewhere'}, root)
    check(False, 'foreign staged path must be refused')
except ns['Refused'] as error:
    check(str(error) == 'fresh_start_staged_path_invalid', str(error))


# The waiting unit: retry while the loader refuses, accept, launch once.
def start(helper_results, launch_code, safe_after=0.0, deadline=60):
    (directory / 'arguments.json').write_bytes(ns['canonical']({**args, 'deadlineSeconds': deadline}))
    (directory / 'boot').write_bytes(b'private-boot-credential')
    calls = []
    def run(argv, **_):
        calls.append(argv[1] if argv[0] == '/bin/sh' else 'helper')
        if argv[0] == '/bin/sh':
            return Result(launch_code, '')
        return helper_results.pop(0) if len(helper_results) > 1 else helper_results[0]
    began = time.monotonic()
    ns['path_safe'] = lambda path, owner=0: time.monotonic() - began >= safe_after
    class NoEvents:
        def wait(self, seconds): time.sleep(seconds)
    ns['run_start'](directory, watch=NoEvents(), run=run)
    record = json.loads((directory / 'status.json').read_bytes())
    check(not (directory / 'boot').exists(), 'credential removed after the start')
    return record, calls

record, calls = start([unsafe, unsafe, Result(0, json.dumps(ready))], 0)
check(record['status'] == 'launched' and record['receipt']['status'] == 'ready', f'launched: {record}')
check(calls == ['helper', 'helper', 'helper', '-c'], f'calls {calls}')
record, calls = start([Result(0, json.dumps(ready))], 0, safe_after=0.4)
check(record['status'] == 'launched' and calls[0] == 'helper', 'waits for the path, then starts')
record, calls = start([Result(0, json.dumps(ready))], 78)
check(record['status'] == 'rejected' and record['code'] == 'fresh_start_runtime_hash_mismatch', f'{record}')
bad = json.loads(json.dumps(ready)); bad['resident']['childAlive'] = True
record, calls = start([Result(0, json.dumps(bad))], 0)
check(record['status'] == 'rejected' and '-c' not in calls, 'invalid receipt never launches')
record, calls = start([Result(0, json.dumps(ready))], 0, safe_after=999, deadline=5)
check(record['status'] == 'timeout' and calls == [], f'deadline: {record} {calls}')
print('ok')
