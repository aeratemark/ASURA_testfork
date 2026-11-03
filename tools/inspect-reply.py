"""Extract the reference's message stream structure from a captured reply.

Reads .gwps-reply.bin (written by `cargo run -p asura-auth --example probe` against GWPS) and walks
the stream with the *variable-length* rule the reference actually uses: u16 counts, no padding. Prints
each message's offset, header, size and the counted fields of interest, so the layouts in this project
can be checked against reality instead of against the table's maxima.
"""

path = r"C:\Users\Apo\Documents\GitHub\ASURA\.gwps-reply.bin"
data = open(path, "rb").read()
print("stream: %d bytes" % len(data))

position = 0
while position + 2 <= len(data):
    header = int.from_bytes(data[position:position + 2], "little")
    print()
    print("offset 0x%04X  header 0x%04X" % (position, header))
    body = position + 2

    if header == 0x0001:  # session info: two dwords
        size = 2 + 8
    elif header == 0x0007:  # character info: req_id, uuid, unk0, name, extended
        req = data[body:body + 4]
        uuid = data[body + 4:body + 20]
        unk0 = data[body + 20:body + 24]
        name_at = body + 24
        name_count = int.from_bytes(data[name_at:name_at + 2], "little")
        name_at += 2
        name = data[name_at:name_at + name_count * 2].decode("utf-16-le", "replace")
        ext_at = name_at + name_count * 2
        ext_count = int.from_bytes(data[ext_at:ext_at + 2], "little")
        ext_at += 2
        extended = data[ext_at:ext_at + ext_count]
        print("  req_id=%d unk0=%s name=%r n_extended=%d" % (
            int.from_bytes(req, "little"), unk0.hex(), name, ext_count))
        print("  extended = %s" % extended.hex())
        size = 2 + 4 + 16 + 4 + 2 + name_count * 2 + 2 + ext_count
    elif header == 0x0016:  # account settings: req_id, u16 n_data, data
        req = int.from_bytes(data[body:body + 4], "little")
        n = int.from_bytes(data[body + 4:body + 6], "little")
        print("  req_id=%d n_data=%d" % (req, n))
        size = 2 + 4 + 2 + n
    elif header == 0x0014:  # friend stream end
        size = 2 + 8
    elif header == 0x0011:  # account info: walk the fixed part then the u16-counted array
        req = int.from_bytes(data[body:body + 4], "little")
        account = data[body + 28:body + 44]
        character = data[body + 44:body + 60]
        tail = data[body + 60:body + 77]
        print("  req_id=%d account=%s character=%s" % (req, account.hex(), character.hex()))
        print("  tail after the two uuids: %s" % tail.hex())
        n_features = int.from_bytes(data[body + 64:body + 66], "little")
        print("  h003E=%d n_features(u16)=%d features=%s" % (
            int.from_bytes(data[body + 60:body + 64], "little"), n_features,
            data[body + 66:body + 66 + n_features].hex()))
        size = 2 + 4 + 4 + 4 + 8 + 8 + 16 + 16 + 4 + 2 + n_features + 1 + 4
    elif header == 0x0003:  # request response
        req = int.from_bytes(data[body:body + 4], "little")
        status = int.from_bytes(data[body + 4:body + 8], "little")
        print("  req_id=%d status=%d" % (req, status))
        size = 2 + 8
    else:
        print("  unknown to this script; stopping")
        break

    print("  size = %d, ends at 0x%04X" % (size, position + size))
    position += size
