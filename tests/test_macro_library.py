"""宏库: 分组、跨文件引用、use/times 复用、yaml 锚点、绑定处覆盖、报错。"""

import tempfile
import unittest
import warnings
from pathlib import Path

from config_loader import load_config
from hub_macro import SLOT_BASE
from hub_profile import from_config
from macro_library import MacroLibrary, build_library
from tests.test_macro import CAP

REPO = Path(__file__).parent.parent


class Base(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)

    def write(self, name, text):
        p = self.dir / name
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text, encoding="utf-8")
        return p

    def lib(self, **files):
        lib = MacroLibrary()
        for name, text in files.items():
            lib.load_file(self.write(name + ".yaml", text))
        lib.validate_all()
        return lib


class Loading(Base):
    def test_group_defaults_to_file_stem(self):
        lib = self.lib(fps="macros:\n  a:\n    steps: [ { tap: a } ]\n")
        self.assertEqual(lib.names(), ["fps.a"])

    def test_explicit_group_name_and_shorthand_list(self):
        lib = self.lib(whatever="group: office\nmacros:\n  x: [ { tap: a } ]\n")
        self.assertEqual(lib.names(), ["office.x"])

    def test_groups_form_with_several_groups_in_one_file(self):
        lib = self.lib(f="groups:\n  g1:\n    a: [ { tap: a } ]\n  g2:\n    a: [ { tap: b } ]\n")
        self.assertEqual(lib.groups(), {"g1": ["a"], "g2": ["a"]})

    def test_duplicate_across_files_is_an_error_naming_both(self):
        with self.assertRaises(ValueError) as cm:
            self.lib(a="group: g\nmacros:\n  m: [ { tap: a } ]\n", b="group: g\nmacros:\n  m: [ { tap: b } ]\n")
        self.assertIn("重复", str(cm.exception))

    def test_glob_and_missing_glob(self):
        self.write("m/one.yaml", "macros:\n  a: [ { tap: a } ]\n")
        self.write("m/two.yaml", "macros:\n  b: [ { tap: b } ]\n")
        lib = build_library(["m/*.yaml"], self.dir)
        self.assertEqual(lib.names(), ["one.a", "two.b"])
        with self.assertRaises(ValueError):
            build_library(["nope/*.yaml"], self.dir)

    def test_bad_files_are_rejected(self):
        for text in (
            "macros:\n  a:\n    steps: []\n",                                  # 空 steps
            "macros:\n  a:\n    steps: [ { tap: a } ]\n    typo: 1\n",         # 不认识的键
            "macros:\n  a:\n    start: forever\n    steps: [ { tap: a } ]\n",
            "macros:\n  a:\n    repeat: 0\n    steps: [ { tap: a } ]\n",
            "macros:\n  \"a.b\": [ { tap: a } ]\n",                              # 名字含点
            "oops: 1\nmacros:\n  a: [ { tap: a } ]\n",                           # 不认识的顶层键
            "groups: {g: {a: [ { tap: a } ]}}\nmacros: {}\n",                    # groups 与 macros 混用
            "macros:\n  a: [ { tap: a, delay_ms: 51 } ]\n",                      # 奇数延迟: validate 时报
        ):
            with self.subTest(text=text), self.assertRaises(ValueError):
                self.lib(bad=text)


class Reuse(Base):
    def test_default_delay_is_per_macro(self):
        lib = self.lib(g="""macros:
  slow: { default_delay_ms: 100, steps: [ { tap: a } ] }
  outer: { default_delay_ms: 20, steps: [ { tap: b }, { use: slow } ] }
""")
        steps = lib.resolve("g.outer").steps
        self.assertEqual([s["delay_ms"] for s in steps], [20, 100])       # 被引用的宏保持自己的默认延迟

    def test_use_times_and_same_group_short_name(self):
        lib = self.lib(g="macros:\n  a: [ { tap: a } ]\n  b: [ { use: a, times: 3 } ]\n")
        self.assertEqual(len(lib.resolve("g.b").steps), 3)

    def test_cross_group_reference(self):
        lib = self.lib(g1="macros:\n  a: [ { tap: a } ]\n", g2="macros:\n  b: [ { use: g1.a } ]\n")
        self.assertEqual(len(lib.resolve("g2.b").steps), 1)

    def test_unqualified_unique_name_resolves_across_groups(self):
        lib = self.lib(g1="macros:\n  only_here: [ { tap: a } ]\n", g2="macros:\n  b: [ { use: only_here } ]\n")
        self.assertEqual(len(lib.resolve("g2.b").steps), 1)

    def test_ambiguous_name_is_an_error(self):
        lib = MacroLibrary()
        lib.load_file(self.write("g1.yaml", "macros:\n  m: [ { tap: a } ]\n"))
        lib.load_file(self.write("g2.yaml", "macros:\n  m: [ { tap: b } ]\n"))
        with self.assertRaises(ValueError) as cm:
            lib.resolve("m")
        self.assertIn("歧义", str(cm.exception))

    def test_unknown_reference_suggests_close_names(self):
        lib = self.lib(g="macros:\n  rapid_fire: [ { tap: a } ]\n")
        with self.assertRaises(ValueError) as cm:
            lib.resolve("g.rapid_fir")
        self.assertIn("g.rapid_fire", str(cm.exception))

    def test_cycles_are_detected(self):
        with self.assertRaises(ValueError) as cm:
            self.lib(g="macros:\n  a: [ { use: b } ]\n  b: [ { use: a } ]\n")
        self.assertIn("循环", str(cm.exception))

    def test_self_reference_is_a_cycle(self):
        with self.assertRaises(ValueError):
            self.lib(g="macros:\n  a: [ { use: a } ]\n")

    def test_yaml_anchors_and_flattening(self):
        lib = self.lib(g="""x-click: &click
  - { down: mouse_left, delay_ms: 20 }
  - { up: mouse_left, delay_ms: 20 }
macros:
  three: { steps: [ *click, *click, *click ] }
  mixed: { steps: [ *click, { tap: a, delay_ms: 50 } ] }
""")
        self.assertEqual(len(lib.resolve("g.three").steps), 6)
        self.assertEqual(len(lib.resolve("g.mixed").steps), 3)

    def test_bad_use_steps(self):
        for step in ("{ use: a, typo: 1 }", "{ use: a, times: 0 }", "{ use: a, times: 256 }", "{ use: missing }"):
            with self.subTest(step=step), self.assertRaises(ValueError):
                self.lib(g=f"macros:\n  a: [ {{ tap: a }} ]\n  b: [ {step} ]\n")

    def test_runaway_expansion_is_stopped(self):
        with self.assertRaises(ValueError):
            self.lib(g="macros:\n  a: [ { tap: a } ]\n  b: [ { use: a, times: 255 } ]\n  c: [ { use: b, times: 255 } ]\n"
                     "  d: [ { use: c, times: 255 } ]\n")


class Binding(Base):
    def test_ref_binding_equals_capture(self):
        # 宏库里的 cap.tap_a (默认延迟 50) 绑到前进键, 必须和 macro_key_A 抓包逐字节一致
        lib = self.lib(cap="macros:\n  tap_a: [ { tap: a } ]\n")
        seq = from_config("t", {"buttons": {"side_front": {"type": "macro", "ref": "cap.tap_a"}}}, lib).build_packets()
        self.assertEqual((seq[5], seq[6:]), (CAP["A"][1], CAP["A"][2:4]))

    def test_same_macro_on_different_buttons_uses_each_buttons_slot(self):
        lib = self.lib(cap="macros:\n  tap_a: [ { tap: a } ]\n")
        cfg = {"buttons": {b: {"type": "macro", "ref": "tap_a"} for b in ("side_back", "side_front", 0)}}
        p = from_config("t", cfg, lib).profile
        self.assertEqual(sorted(p.macros), [0, 6, 7])
        self.assertEqual({p.macros[i].name for i in p.macros}, {"cap.tap_a"})

    def test_macro_own_start_and_repeat_are_used_and_can_be_overridden(self):
        lib = self.lib(g="macros:\n  m: { start: hold, steps: [ { tap: a } ] }\n  n: { repeat: 4, steps: [ { tap: a } ] }\n")
        p = from_config("t", {"buttons": {6: {"type": "macro", "ref": "g.m"}, 7: {"type": "macro", "ref": "g.n"}}}, lib).profile
        self.assertEqual((p.macros[6].mode, p.macros[7].repeat), (2, 4))
        p = from_config("t", {"buttons": {6: {"type": "macro", "ref": "g.m", "start": "loop", "repeat": 9}}}, lib).profile
        self.assertEqual((p.macros[6].mode, p.macros[6].repeat), (0, 9))

    def test_ref_without_library_or_with_steps_is_rejected(self):
        with self.assertRaises(ValueError):
            from_config("t", {"buttons": {7: {"type": "macro", "ref": "g.m"}}})
        lib = self.lib(g="macros:\n  m: [ { tap: a } ]\n")
        with self.assertRaises(ValueError):
            from_config("t", {"buttons": {7: {"type": "macro", "ref": "g.m", "steps": [{"tap": "a"}]}}}, lib)

    def test_buttons_without_slots_still_rejected_with_ref(self):
        lib = self.lib(g="macros:\n  m: [ { tap: a } ]\n")
        with self.assertRaises(ValueError):
            from_config("t", {"buttons": {4: {"type": "macro", "ref": "g.m"}}}, lib)

    def test_load_config_with_macro_files(self):
        self.write("macros/fps.yaml", "macros:\n  m: [ { tap: a } ]\n")
        cfgfile = self.write("c.yaml", """
device: {vendor_id: 1, product_id: 2, usage_page: 3, usage: 1}
macro_files: [ "macros/*.yaml" ]
profiles:
  p: { buttons: { side_front: { type: macro, ref: fps.m } } }
rules: [ { process: "*", profile: p } ]
""")
        cfg = load_config(cfgfile)
        self.assertEqual(cfg.macro_library.names(), ["fps.m"])
        self.assertEqual(len(cfg.profiles["p"].build_packets()), 8)


class RepoExamples(unittest.TestCase):
    def test_shipped_config_and_macro_files_load(self):
        cfg = load_config(REPO / "config.hub.yaml")
        self.assertIn("fps.rapid_fire", cfg.macro_library.names())
        d = cfg.macro_library.resolve("fps.rapid_fire")
        self.assertEqual((d.start, len(d.steps)), ("hold", 2))

    def test_every_shipped_macro_binds_and_encodes(self):
        lib = build_library(["macros/*.yaml"], REPO)
        with warnings.catch_warnings():
            warnings.simplefilter("error")
            for fq in lib.names():
                p = from_config("t", {"buttons": {"side_front": {"type": "macro", "ref": fq}}}, lib)
                self.assertGreaterEqual(len(p.build_packets()), 8, fq)

    def test_live_profiles_unchanged(self):
        cfg = load_config(REPO / "config.hub.yaml")
        self.assertEqual(cfg.profiles["cf"].current_buttons()["side_front"], "5")
        self.assertEqual(cfg.profiles["default"].profile.light_mode, 0)
        self.assertEqual(cfg.profiles["cf"].profile.light_mode, 4)


if __name__ == "__main__":
    unittest.main()
