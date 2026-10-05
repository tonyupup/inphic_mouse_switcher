"""标准 USB HID 键盘 Usage Table，键名 -> 扫描码。只收录常用的单键。"""

HID_KEYCODES = {
    **{chr(c): 0x04 + (c - ord('a')) for c in range(ord('a'), ord('z') + 1)},
    "1": 0x1E, "2": 0x1F, "3": 0x20, "4": 0x21, "5": 0x22,
    "6": 0x23, "7": 0x24, "8": 0x25, "9": 0x26, "0": 0x27,
    "enter": 0x28, "esc": 0x29, "backspace": 0x2A, "tab": 0x2B, "space": 0x2C,
    "-": 0x2D, "=": 0x2E, "[": 0x2F, "]": 0x30, "\\": 0x31,
    ";": 0x33, "'": 0x34, "`": 0x35, ",": 0x36, ".": 0x37, "/": 0x38,
    "capslock": 0x39,
    "f1": 0x3A, "f2": 0x3B, "f3": 0x3C, "f4": 0x3D, "f5": 0x3E, "f6": 0x3F,
    "f7": 0x40, "f8": 0x41, "f9": 0x42, "f10": 0x43, "f11": 0x44, "f12": 0x45,
    "left_ctrl": 0xE0, "left_shift": 0xE1, "left_alt": 0xE2, "left_win": 0xE3,
    "printscreen": 0x46, "scrolllock": 0x47, "pause": 0x48,
    "insert": 0x49, "home": 0x4A, "pageup": 0x4B, "delete": 0x4C, "end": 0x4D, "pagedown": 0x4E,
    "right": 0x4F, "left": 0x50, "down": 0x51, "up": 0x52,
    "numlock": 0x53, "kp_divide": 0x54, "kp_multiply": 0x55, "kp_minus": 0x56, "kp_plus": 0x57,
    "kp_enter": 0x58, "kp_1": 0x59, "kp_2": 0x5A, "kp_3": 0x5B, "kp_4": 0x5C, "kp_5": 0x5D,
    "kp_6": 0x5E, "kp_7": 0x5F, "kp_8": 0x60, "kp_9": 0x61, "kp_0": 0x62, "kp_period": 0x63,
    "menu": 0x65,
    **{f"f{n}": 0x68 + (n - 13) for n in range(13, 25)},
    "right_ctrl": 0xE4, "right_shift": 0xE5, "right_alt": 0xE6, "right_win": 0xE7,
}


def key_to_code(key: str) -> int:
    k = key.strip().lower()
    if k not in HID_KEYCODES:
        raise ValueError(f"不认识的键名: {key!r}，可以在 hid_keycodes.py 里加")
    return HID_KEYCODES[k]


def code_to_key(code: int) -> str:
    for name, c in HID_KEYCODES.items():
        if c == code:
            return name
    return f"未知键码 {code:#04x}"
