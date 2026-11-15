# Copies a PE executable with its subsystem set to console (3), like `editbin /SUBSYSTEM:CONSOLE`.
import struct, sys
data = bytearray(open(sys.argv[1], 'rb').read())
pe = struct.unpack_from('<I', data, 0x3C)[0]
assert data[pe:pe+4] == b'PE\0\0'
opt = pe + 24
assert struct.unpack_from('<H', data, opt)[0] == 0x20B  # PE32+
off = opt + 68
assert struct.unpack_from('<H', data, off)[0] == 2, 'not a GUI exe'
struct.pack_into('<H', data, off, 3)
open(sys.argv[2], 'wb').write(data)
