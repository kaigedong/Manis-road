#!/usr/bin/env python3
"""Collect ONLY MotuHTTPS records, reassembling Android log chunks as JSONL."""
import argparse
import json
import os
import re
import subprocess
from datetime import datetime, timezone
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, default=Path('var/motu/logs') / (datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ') + '.jsonl'))
    parser.add_argument('--serial')
    parser.add_argument('--dump', action='store_true', help='Read existing log buffer then exit')
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    cmd = ['adb'] + (['-s', args.serial] if args.serial else []) + ['logcat', '-v', 'raw']
    if args.dump:
        cmd.append('-d')
    cmd.extend(['MotuHTTPS:I', '*:S'])
    pending = {}
    fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    print(f'Writing {args.output}', flush=True)
    with os.fdopen(fd, 'w') as output:
        proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, text=True, encoding='utf-8', errors='replace')
        try:
            for line in proc.stdout:
                match = re.match(r'^(\d+-\d+) (\d+)/(\d+) (.*)$', line.rstrip('\n'))
                if not match:
                    continue
                key, part, total, text = match.groups()
                if key not in pending and len(pending) >= 1000:
                    pending.pop(next(iter(pending)))
                entry = pending.setdefault(key, {})
                entry[int(part)] = text
                if len(entry) == int(total):
                    joined = ''.join(entry[i] for i in range(1, int(total) + 1))
                    try:
                        record = json.loads(joined)
                        output.write(json.dumps(record, ensure_ascii=False) + '\n')
                        output.flush()
                        print(record.get('event'), record.get('method', ''), record.get('url', ''), flush=True)
                    except (ValueError, KeyError):
                        print(f'Invalid/incomplete record: {key}', flush=True)
                    del pending[key]
        except KeyboardInterrupt:
            proc.terminate()
        finally:
            if proc.poll() is None:
                proc.terminate()
            proc.wait()
            if pending:
                print(f'{len(pending)} incomplete records (log buffer may have dropped chunks)')


if __name__ == '__main__':
    main()
