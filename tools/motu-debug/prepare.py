#!/usr/bin/env python3
"""Build a separate manifest without rewriting the protected primary DEX."""
import argparse
import hashlib
import json
import lzma
import struct
import zipfile
from pathlib import Path

ORIGINAL = 'com.motu.motumap'
DEBUG = 'com.motu.debug'


def manifest(data):
    # The vendor's XML header contains four extra bytes; parse its actual header size.
    header_size = struct.unpack_from('<H', data, 2)[0]
    pool = header_size
    kind, header, size, count, styles, flags, start, style_start = struct.unpack_from('<HH6I', data, pool)
    if (kind, header, styles, flags) != (1, 28, 0, 0):
        raise ValueError('Unsupported manifest string pool')
    strings = []
    for i in range(count):
        pos = pool + start + struct.unpack_from('<I', data, pool + 28 + i * 4)[0]
        length = struct.unpack_from('<H', data, pos)[0]
        if length & 0x8000:
            raise ValueError('Unexpected long string')
        strings.append(data[pos + 2:pos + 2 + length * 2].decode('utf-16le'))

    def intern(text):
        if text not in strings:
            strings.append(text)
        return strings.index(text)

    tail = bytearray(data[pool + size:])
    offset = 0
    changed = []
    while offset < len(tail):
        typ, _, length = struct.unpack_from('<HHI', tail, offset)
        if length < 8 or offset + length > len(tail):
            raise ValueError('Invalid XML chunk')
        if typ == 0x102:
            _, name, attr_start, attr_size, attrs = struct.unpack_from('<IIHHH', tail, offset + 16)
            element = strings[name]
            for i in range(attrs):
                a = offset + 16 + attr_start + i * attr_size
                _, attr_name, raw, _, _, value_type, value = struct.unpack_from('<IIIHBBI', tail, a)
                attr = strings[attr_name]
                if value_type != 3:
                    continue
                old = strings[value]
                new = old
                if element == 'manifest' and attr == 'package':
                    new = DEBUG
                elif element == 'application' and attr == 'name':
                    new = 'lab.motu.DebugApplication'
                elif element == 'application' and attr == 'label':
                    new = '摩途 HTTPS 调试'
                elif attr in ('authorities', 'permission', 'readPermission', 'writePermission') or element in ('permission', 'uses-permission'):
                    new = old.replace(ORIGINAL, DEBUG)
                if new != old:
                    index = intern(new)
                    struct.pack_into('<I', tail, a + 16, index)
                    if raw != 0xffffffff:
                        struct.pack_into('<I', tail, a + 8, index)
                    changed.append([element, attr, old, new])
        offset += length
    offsets, encoded = [], bytearray()
    for text in strings:
        raw = text.encode('utf-16le')
        offsets.append(len(encoded))
        encoded += struct.pack('<H', len(raw) // 2) + raw + b'\0\0'
    encoded += b'\0' * (-len(encoded) % 4)
    start = 28 + 4 * len(strings)
    string_pool = struct.pack('<HH6I', 1, 28, start + len(encoded), len(strings), 0, 0, start, 0)
    string_pool += struct.pack(f'<{len(offsets)}I', *offsets) + encoded
    return struct.pack('<HHI', 3, 8, 8 + len(string_pool) + len(tail)) + string_pool + tail, changed


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--work', type=Path, default=Path('var/motu'))
    args = parser.parse_args()
    work = args.work
    output = work / 'build'
    output.mkdir(exist_ok=True)
    source = work / 'original/motu-device.apk'
    with zipfile.ZipFile(source) as z:
        xml, changes = manifest(z.read('AndroidManifest.xml'))
        with zipfile.ZipFile(output / 'motu-debug-unsigned.apk', 'w') as out:
            for info in z.infolist():
                if info.filename.startswith('META-INF/'):
                    continue
                out.writestr(info, xml if info.filename == 'AndroidManifest.xml' else z.read(info.filename))
            for target, path in [
                ('classes2.dex', output / 'classes2.dex'),
                ('assets/motu-network.js', work / 'tools/js/network.bundle.js'),
            ]:
                out.write(path, target, compress_type=zipfile.ZIP_DEFLATED)
            out.writestr('lib/arm64-v8a/libmotulog.so', lzma.decompress((work / 'tools/frida-gadget.so.xz').read_bytes()), compress_type=zipfile.ZIP_STORED)
            config = {'interaction': {'type': 'script', 'path': f'/data/user/0/{DEBUG}/files/motu-network.js', 'on_change': 'ignore'}}
            out.writestr('lib/arm64-v8a/libmotulog.config.so', json.dumps(config), compress_type=zipfile.ZIP_STORED)
    (output / 'manifest-changes.json').write_text(json.dumps(changes, ensure_ascii=False, indent=2))
    (output / 'source-sha256.txt').write_text(hashlib.sha256(source.read_bytes()).hexdigest() + '\n')
    print(output / 'motu-debug-unsigned.apk')


if __name__ == '__main__':
    main()
