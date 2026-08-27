# SPDX-License-Identifier: MIT
import struct
import sys


def parse_mpls(filepath):
    with open(filepath, 'rb') as f:
        data = f.read()

    playlist_pos = struct.unpack('>I', data[8:12])[0]
    pos = playlist_pos
    num_playitems = struct.unpack('>H', data[pos + 6:pos + 8])[0]

    pos += 10
    playitems = []

    for i in range(num_playitems):
        item_len = struct.unpack('>H', data[pos:pos + 2])[0]
        item_start = pos
        clip_name = data[pos + 2:pos + 7].decode('ascii', errors='ignore')
        in_time = struct.unpack('>I', data[pos + 14:pos + 18])[0]
        out_time = struct.unpack('>I', data[pos + 18:pos + 22])[0]
        duration_sec = (out_time - in_time) / 45000.0

        playitems.append({
            'index': i + 1,
            'clip': clip_name,
            'in_sec': in_time / 45000.0,
            'out_sec': out_time / 45000.0,
            'duration': duration_sec
        })
        pos = item_start + 2 + item_len

    return playitems


if __name__ == "__main__":
    if len(sys.argv) > 1:
        items = parse_mpls(sys.argv[1])
        for it in items:
            print(
                f"Item {
                    it['index']:02d}: Clip {
                    it['clip']}.m2ts | Duration: {
                    it['duration']:.3f}s")
