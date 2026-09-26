import sys

def parse_proto(b):
    pos = 0
    fields = {}
    while pos < len(b):
        # read varint
        val = 0
        shift = 0
        while True:
            byte = b[pos]
            pos += 1
            val |= (byte & 0x7f) << shift
            shift += 7
            if not (byte & 0x80):
                break
        field_num = val >> 3
        wire_type = val & 7
        if wire_type == 0:
            # varint
            v = 0
            s = 0
            while True:
                byte = b[pos]
                pos += 1
                v |= (byte & 0x7f) << s
                s += 7
                if not (byte & 0x80):
                    break
            fields[field_num] = ('varint', v)
        elif wire_type == 2:
            # length delimited
            length = 0
            s = 0
            while True:
                byte = b[pos]
                pos += 1
                length |= (byte & 0x7f) << s
                s += 7
                if not (byte & 0x80):
                    break
            data = b[pos:pos+length]
            pos += length
            fields[field_num] = ('bytes', data)
        else:
            print(f"Unsupported wire type {wire_type}")
            break
    return fields

print("Decoder ready")
