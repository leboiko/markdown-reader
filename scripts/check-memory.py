#!/usr/bin/env python3
"""Build optimized memory workloads, sample RSS, then run macOS leaks.
Run from any directory: python3 scripts/check-memory.py
Requires cargo and macOS leaks/ps. Exits nonzero on any failed gate.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading

os.chdir(Path(__file__).resolve().parent.parent)
build = subprocess.run(
    ['cargo', 'test', '--release', '--bin', 'markdown-reader', '--no-run',
     '--message-format=json'], capture_output=True, text=True, check=True)
executables = [item['executable'] for line in build.stdout.splitlines()
               if (item := json.loads(line)).get('executable')
               and item.get('profile', {}).get('test')]
assert len(executables) == 1, executables
command = [executables[0], 'memory_soak', '--ignored', '--nocapture', '--test-threads=1']
with tempfile.TemporaryFile(mode='w+') as output:
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=output, text=True)
    samples = []
    def collect():
        for line in process.stdout:
            print(line, end='', flush=True)
            if line.startswith('MEMORY_SAMPLE'):
                result = subprocess.run(['ps', '-o', 'rss=', '-p', str(process.pid)],
                                        capture_output=True, text=True, check=True)
                samples.append(int(result.stdout.strip()))
    reader = threading.Thread(target=collect)
    reader.start()
    status = process.wait()
    reader.join()
    output.seek(0)
    sys.stdout.write(output.read())
    if status:
        sys.exit(status)
assert len(samples) >= 20, 'missing RSS checkpoints'
# Exclude initial parser/font/allocator warmup. Check sustained growth and peak.
steady = samples[3:]
growth = max(steady) - min(steady)
print(f'RSS KiB: checkpoints={samples}; steady spread={growth}', flush=True)
assert growth <= 16384, f'RSS grew by {growth} KiB (budget 16384 KiB)'
if sys.platform == 'darwin':
    env = dict(os.environ, MallocStackLogging='1')
    subprocess.run(['leaks', '--noContent', '--atExit', '--', *command],
                   env=env, check=True)
else:
    raise SystemExit('Native leaks gate requires macOS; use AddressSanitizer elsewhere')
