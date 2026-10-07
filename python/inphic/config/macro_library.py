"""
宏库: 宏按"分组.名字"独立管理, 与按钮无关; 按钮只用 `ref` 引用。可以拆成多个 yaml 文件。

文件格式(任选其一):

    # 单分组文件: 分组名默认取文件名
    group: fps                    # 可省略
    macros:
      click_burst:
        description: 连点
        start: loop               # loop | stop_on_any_key | hold | stop_on_this_key   (默认 loop)
        repeat: 5                 # 仅 loop 有意义                                      (默认 1)
        default_delay_ms: 20      # 本宏里没写 delay_ms 的步骤用它                       (默认 50)
        steps:
          - { tap: mouse_left }
          - { use: common.release_all, times: 2 }      # 展开别的宏
      short: [ { tap: a } ]       # 简写: 只有步骤列表

    # 多分组文件
    groups:
      fps:    { click_burst: {...}, ... }
      office: { copy_paste: {...} }

复用:
    - `use: 分组.名字` 把另一个宏的步骤展开到这里(可选 `times: N`); 同分组内可以只写名字。
      被引用的宏用它自己的 default_delay_ms; 不能循环引用。
    - YAML 锚点 `&x` / `*x` 照常可用; `steps` 里嵌套的列表会被摊平, 所以可以 `steps: [*common_steps, {tap: x}]`;
      锚点定义放在以 `x-` 开头的顶层键下(如 `x-click: &click [...]`), 不会被当成配置。
    - 绑定处覆盖: `{ type: macro, ref: fps.click_burst, start: hold, repeat: 3 }`。
"""

from __future__ import annotations

import difflib
import glob
from dataclasses import dataclass
from pathlib import Path

import yaml

from inphic.protocol.macro import START_MODES

_DEF_KEYS = {"steps", "start", "repeat", "default_delay_ms", "description"}
_USE_KEYS = {"use", "times"}
_ACTION_KEYS = ("down", "up", "tap", "move")
DEFAULT_DELAY_MS = 50
MAX_USE_TIMES = 255
MAX_EXPANDED_STEPS = 5000


@dataclass(frozen=True)
class MacroDef:
    """展开后的宏: steps 里每一步都是完整的、带显式 delay_ms 的普通步骤, 可直接交给 protocol.macro.parse_steps。"""
    name: str                  # "分组.名字"
    steps: tuple
    start: str = "loop"
    repeat: int = 1
    description: str = ""
    source: str = ""           # 定义它的文件, 报错时用


def _flatten(steps):
    for s in steps:
        if isinstance(s, (list, tuple)):
            yield from _flatten(s)
        else:
            yield s


def _check_name(kind: str, name: str, where: str) -> None:
    if not isinstance(name, str) or not name or "." in name or name != name.strip():
        raise ValueError(f"{where}: {kind}名 {name!r} 不合法(不能为空、不能含 '.'、不能有首尾空格)")


class MacroLibrary:
    def __init__(self):
        self._raw: dict[str, tuple[dict, str]] = {}      # "分组.名字" -> (原始定义, 来源文件)

    # ---------------- 加载 ----------------
    def load_file(self, path) -> int:
        path = Path(path)
        try:
            data = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        except yaml.YAMLError as e:
            raise ValueError(f"{path}: yaml 解析失败: {e}") from None
        if not isinstance(data, dict):
            raise ValueError(f"{path}: 顶层需要是字典")
        # 以 x- 或 . 开头的顶层键留给 yaml 锚点定义(惯例), 不当作配置
        extra = {k for k in data if k not in ("group", "macros", "groups")
                 and not (isinstance(k, str) and k.startswith(("x-", ".")))}
        if extra:
            raise ValueError(f"{path}: 顶层有不认识的键 {sorted(extra)}, 可选: group, macros, groups")
        if "groups" in data:
            if "group" in data or "macros" in data:
                raise ValueError(f"{path}: groups 不能和 group/macros 同时出现")
            groups = data["groups"] or {}
        else:
            groups = {data.get("group", path.stem): data.get("macros") or {}}
        count = 0
        for gname, macros in groups.items():
            _check_name("分组", gname, str(path))
            if not isinstance(macros, dict):
                raise ValueError(f"{path}: 分组 {gname!r} 里需要是 名字: 定义 的字典")
            for mname, raw in macros.items():
                _check_name("宏", mname, f"{path} 分组 {gname}")
                fq = f"{gname}.{mname}"
                if fq in self._raw:
                    raise ValueError(f"宏 {fq} 重复定义: {self._raw[fq][1]} 和 {path}")
                self._raw[fq] = (self._normalize(raw, fq, path), str(path))
                count += 1
        return count

    def load_patterns(self, patterns, base_dir) -> int:
        """patterns: 文件名或通配符列表, 相对 base_dir。通配符没匹配到任何文件时报错(通常是路径写错)。"""
        total = 0
        for pat in patterns or []:
            full = Path(pat) if Path(pat).is_absolute() else Path(base_dir) / pat
            files = sorted(glob.glob(str(full)))
            if not files:
                raise ValueError(f"macro_files 里的 {pat!r} 没有匹配到任何文件(相对 {base_dir})")
            for f in files:
                total += self.load_file(f)
        self.validate_all()
        return total

    @staticmethod
    def _normalize(raw, fq, path) -> dict:
        if isinstance(raw, list):
            raw = {"steps": raw}
        if not isinstance(raw, dict):
            raise ValueError(f"{path}: 宏 {fq} 的定义需要是字典或步骤列表")
        extra = set(raw) - _DEF_KEYS
        if extra:
            raise ValueError(f"{path}: 宏 {fq} 里有不认识的键 {sorted(extra)}, 可选: {sorted(_DEF_KEYS)}")
        if not raw.get("steps"):
            raise ValueError(f"{path}: 宏 {fq} 需要非空的 steps")
        start = raw.get("start", "loop")
        if start not in START_MODES:
            raise ValueError(f"{path}: 宏 {fq} 的 start {start!r} 不认识, 可选: {sorted(START_MODES)}")
        rep = raw.get("repeat", 1)
        if not isinstance(rep, int) or isinstance(rep, bool) or not 1 <= rep <= 255:
            raise ValueError(f"{path}: 宏 {fq} 的 repeat {rep!r} 需要是 1..255 的整数")
        d = raw.get("default_delay_ms", DEFAULT_DELAY_MS)
        if not isinstance(d, int) or isinstance(d, bool):
            raise ValueError(f"{path}: 宏 {fq} 的 default_delay_ms 需要是整数")
        return raw

    # ---------------- 查询 ----------------
    def names(self) -> list[str]:
        return sorted(self._raw)

    def groups(self) -> dict[str, list[str]]:
        out: dict[str, list[str]] = {}
        for fq in self.names():
            g, n = fq.split(".", 1)
            out.setdefault(g, []).append(n)
        return out

    def _lookup(self, ref: str, group: str | None) -> str:
        if not isinstance(ref, str) or not ref:
            raise ValueError(f"宏引用 {ref!r} 不合法")
        if "." in ref:
            if ref in self._raw:
                return ref
        else:
            if group and f"{group}.{ref}" in self._raw:           # 同分组优先
                return f"{group}.{ref}"
            hits = [fq for fq in self._raw if fq.endswith("." + ref)]
            if len(hits) == 1:
                return hits[0]
            if len(hits) > 1:
                raise ValueError(f"宏引用 {ref!r} 有歧义: {hits}, 请写成 分组.名字")
        close = difflib.get_close_matches(ref, self._raw, n=3)
        hint = f", 你是不是想写: {close}" if close else f", 已有的宏: {self.names() or '(宏库是空的)'}"
        raise ValueError(f"找不到宏 {ref!r}{hint}")

    # ---------------- 展开 ----------------
    def resolve(self, ref: str) -> MacroDef:
        fq = self._lookup(ref, None)
        raw, src = self._raw[fq]
        return MacroDef(
            name=fq, steps=tuple(self._expand(fq, ())), start=raw.get("start", "loop"),
            repeat=raw.get("repeat", 1), description=raw.get("description", ""), source=src,
        )

    def _expand(self, fq: str, stack: tuple) -> list[dict]:
        if fq in stack:
            raise ValueError("宏循环引用: " + " -> ".join(stack + (fq,)))
        raw, src = self._raw[fq]
        group = fq.split(".", 1)[0]
        default = raw.get("default_delay_ms", DEFAULT_DELAY_MS)
        out: list[dict] = []
        for n, step in enumerate(_flatten(raw["steps"]), 1):
            where = f"{src} 宏 {fq} 第 {n} 步"
            if not isinstance(step, dict):
                raise ValueError(f"{where}: 步骤需要是字典, 实际 {step!r}")
            if "use" in step:
                extra = set(step) - _USE_KEYS
                if extra:
                    raise ValueError(f"{where}: use 步骤里有不认识的键 {sorted(extra)}, 可选: use, times")
                times = step.get("times", 1)
                if not isinstance(times, int) or isinstance(times, bool) or not 1 <= times <= MAX_USE_TIMES:
                    raise ValueError(f"{where}: times {times!r} 需要是 1..{MAX_USE_TIMES} 的整数")
                try:
                    target = self._lookup(step["use"], group)
                except ValueError as e:
                    raise ValueError(f"{where}: {e}") from None
                out.extend(self._expand(target, stack + (fq,)) * times)
            else:
                s = dict(step)
                if any(k in s for k in _ACTION_KEYS):
                    s.setdefault("delay_ms", default)         # 本宏的默认延迟在展开时就写死, 引用到别处也不变
                out.append(s)
            if len(out) > MAX_EXPANDED_STEPS:
                raise ValueError(f"{fq} 展开后超过 {MAX_EXPANDED_STEPS} 步, 检查 use/times 是不是写大了")
        return out

    def validate_all(self) -> None:
        """加载完所有文件后调用: 逐个展开并用真正的解析器检查, 配置错误在启动时就暴露而不是用到才报。"""
        from inphic.protocol.macro import parse_steps
        for fq in self.names():
            d = self.resolve(fq)
            try:
                parse_steps(list(d.steps), DEFAULT_DELAY_MS)
            except ValueError as e:
                raise ValueError(f"{d.source} 宏 {fq}: {e}") from None


def build_library(patterns, base_dir) -> MacroLibrary:
    lib = MacroLibrary()
    lib.load_patterns(patterns, base_dir)
    return lib
