//! 宏库: 宏按"分组.名字"独立管理, 与按钮无关; 按钮只用 `ref` 引用。可以拆成多个 yaml 文件。
//!
//! 文件格式(任选其一):
//!
//! ```yaml
//! # 单分组文件: 分组名默认取文件名
//! group: fps                    # 可省略
//! macros:
//!   click_burst:
//!     description: 连点
//!     start: loop               # loop | stop_on_any_key | hold | stop_on_this_key   (默认 loop)
//!     repeat: 5                 # 仅 loop 有意义                                      (默认 1)
//!     default_delay_ms: 20      # 本宏里没写 delay_ms 的步骤用它                       (默认 50)
//!     steps:
//!       - { tap: mouse_left }
//!       - { use: common.release_all, times: 2 }      # 展开别的宏
//!   short: [ { tap: a } ]       # 简写: 只有步骤列表
//!
//! # 多分组文件
//! groups:
//!   fps:    { click_burst: {...}, ... }
//!   office: { copy_paste: {...} }
//! ```
//!
//! 复用:
//! - `use: 分组.名字` 把另一个宏的步骤展开到这里(可选 `times: N`); 同分组内可以只写名字。
//!   被引用的宏用它自己的 default_delay_ms; 不能循环引用。
//! - YAML 锚点 `&x` / `*x` 照常可用; `steps` 里嵌套的列表会被摊平, 所以可以 `steps: [*common_steps, {tap: x}]`;
//!   锚点定义放在以 `x-` 开头的顶层键下(如 `x-click: &click [...]`), 不会被当成配置。
//! - 绑定处覆盖: `{ type: macro, ref: fps.click_burst, start: hold, repeat: 3 }`。

use std::collections::BTreeMap;
use std::path::Path;

use serde_yaml::{Mapping, Value};

use crate::bail;
use crate::error::{fmt_list, Result};
use crate::profile::build::{MacroLibraryLookup, MacroSource};
use crate::protocol::macros::{parse_steps, start_mode, start_mode_names_sorted};
use crate::yamlutil::{as_int, fmt_keys, repr, unknown_keys};

const DEF_KEYS: [&str; 5] = ["steps", "start", "repeat", "default_delay_ms", "description"];
const ACTION_KEYS: [&str; 4] = ["down", "up", "tap", "move"];
pub const DEFAULT_DELAY_MS: i64 = 50;
const MAX_USE_TIMES: i64 = 255;
const MAX_EXPANDED_STEPS: usize = 5000;

/// 展开后的宏: steps 里每一步都是完整的、带显式 delay_ms 的普通步骤, 可直接交给 `parse_steps`。
#[derive(Debug, Clone)]
pub struct MacroDef {
    /// "分组.名字"
    pub name: String,
    pub steps: Vec<Value>,
    pub start: String,
    pub repeat: i64,
    pub description: String,
    /// 定义它的文件, 报错时用
    pub source: String,
}

fn flatten<'a>(steps: &'a [Value], out: &mut Vec<&'a Value>) {
    for s in steps {
        match s {
            Value::Sequence(inner) => flatten(inner, out),
            other => out.push(other),
        }
    }
}

fn check_name(kind: &str, name: &Value, wher: &str) -> Result<String> {
    match name {
        Value::String(s) if !s.is_empty() && !s.contains('.') && s == s.trim() => Ok(s.clone()),
        other => bail!("{wher}: {kind}名 {} 不合法(不能为空、不能含 '.'、不能有首尾空格)", repr(other)),
    }
}

/// 读 yaml 文件, 解析锚点/合并键; 错误统一成 `ValueError`。
pub fn read_yaml(path: &Path) -> Result<Value> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => bail!("读取 {} 失败: {e}", path.display()),
    };
    parse_yaml(&text, path)
}

pub fn parse_yaml(text: &str, path: &Path) -> Result<Value> {
    let mut value: Value = match serde_yaml::from_str(text) {
        Ok(v) => v,
        Err(e) => bail!("{}: yaml 解析失败: {e}", path.display()),
    };
    if let Err(e) = value.apply_merge() {
        bail!("{}: yaml 解析失败: {e}", path.display());
    }
    Ok(value)
}

#[derive(Default, Debug)]
pub struct MacroLibrary {
    /// "分组.名字" -> (原始定义, 来源文件)
    raw: BTreeMap<String, (Mapping, String)>,
}

impl MacroLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    // ---------------- 加载 ----------------
    pub fn load_file(&mut self, path: &Path) -> Result<usize> {
        let data = read_yaml(path)?;
        self.load_value(data, path)
    }

    pub fn load_value(&mut self, data: Value, path: &Path) -> Result<usize> {
        let disp = path.display().to_string();
        let data = match data {
            Value::Null => Mapping::new(),
            Value::Mapping(m) => m,
            _ => bail!("{disp}: 顶层需要是字典"),
        };
        // 以 x- 或 . 开头的顶层键留给 yaml 锚点定义(惯例), 不当作配置
        let mut extra: Vec<String> = data
            .keys()
            .filter(|k| match k {
                Value::String(s) => {
                    !["group", "macros", "groups"].contains(&s.as_str()) && !(s.starts_with("x-") || s.starts_with('.'))
                }
                _ => true,
            })
            .map(|k| match k {
                Value::String(s) => s.clone(),
                other => repr(other),
            })
            .collect();
        extra.sort();
        if !extra.is_empty() {
            bail!("{disp}: 顶层有不认识的键 {}, 可选: group, macros, groups", fmt_keys(&extra));
        }
        let groups: Vec<(Value, Value)> = if data.contains_key("groups") {
            if data.contains_key("group") || data.contains_key("macros") {
                bail!("{disp}: groups 不能和 group/macros 同时出现");
            }
            match &data["groups"] {
                Value::Null => vec![],
                Value::Mapping(g) => g.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                other => bail!("{disp}: groups 需要是 分组名: 宏字典 的字典, 实际 {}", repr(other)),
            }
        } else {
            let gname = match data.get("group") {
                Some(g) => g.clone(),
                None => Value::String(path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()),
            };
            let macros = match data.get("macros") {
                None | Some(Value::Null) => Value::Mapping(Mapping::new()),
                Some(m) => m.clone(),
            };
            vec![(gname, macros)]
        };
        let mut count = 0;
        for (gname, macros) in groups {
            let gname = check_name("分组", &gname, &disp)?;
            let Value::Mapping(macros) = macros else {
                bail!("{disp}: 分组 {gname:?} 里需要是 名字: 定义 的字典");
            };
            for (mname, raw) in macros {
                let mname = check_name("宏", &mname, &format!("{disp} 分组 {gname}"))?;
                let fq = format!("{gname}.{mname}");
                if let Some((_, src)) = self.raw.get(&fq) {
                    bail!("宏 {fq} 重复定义: {src} 和 {disp}");
                }
                self.raw.insert(fq.clone(), (Self::normalize(raw, &fq, &disp)?, disp.clone()));
                count += 1;
            }
        }
        Ok(count)
    }

    /// patterns: 文件名或通配符列表, 相对 base_dir。通配符没匹配到任何文件时报错(通常是路径写错)。
    pub fn load_patterns(&mut self, patterns: &[String], base_dir: &Path) -> Result<usize> {
        let mut total = 0;
        for pat in patterns {
            let full = if Path::new(pat).is_absolute() {
                pat.clone()
            } else {
                // 目录部分按字面量转义, 只有 pattern 自己的通配符生效
                let base = base_dir.to_string_lossy().replace('\\', "/");
                format!("{}/{}", glob::Pattern::escape(base.trim_end_matches('/')), pat.replace('\\', "/"))
            };
            let mut files: Vec<String> = match glob::glob(&full) {
                Ok(paths) => paths.filter_map(|p| p.ok()).map(|p| p.to_string_lossy().into_owned()).collect(),
                Err(e) => bail!("macro_files 里的 {pat:?} 不是合法的通配符: {e}"),
            };
            files.sort();
            if files.is_empty() {
                bail!("macro_files 里的 {pat:?} 没有匹配到任何文件(相对 {})", base_dir.display());
            }
            for f in files {
                total += self.load_file(Path::new(&f))?;
            }
        }
        self.validate_all()?;
        Ok(total)
    }

    fn normalize(raw: Value, fq: &str, path: &str) -> Result<Mapping> {
        let raw = match raw {
            Value::Sequence(_) => {
                let mut m = Mapping::new();
                m.insert("steps".into(), raw);
                m
            }
            Value::Mapping(m) => m,
            _ => bail!("{path}: 宏 {fq} 的定义需要是字典或步骤列表"),
        };
        let extra = unknown_keys(&raw, &DEF_KEYS);
        if !extra.is_empty() {
            let mut keys = DEF_KEYS.to_vec();
            keys.sort();
            bail!("{path}: 宏 {fq} 里有不认识的键 {}, 可选: {}", fmt_keys(&extra), fmt_list(&keys));
        }
        let steps_ok = matches!(raw.get("steps"), Some(Value::Sequence(s)) if !s.is_empty());
        if !steps_ok {
            bail!("{path}: 宏 {fq} 需要非空的 steps");
        }
        if let Some(s) = raw.get("start") {
            if !matches!(s, Value::String(n) if start_mode(n).is_some()) {
                bail!("{path}: 宏 {fq} 的 start {} 不认识, 可选: {}", repr(s), fmt_list(&start_mode_names_sorted()));
            }
        }
        if let Some(r) = raw.get("repeat") {
            if !as_int(r).is_some_and(|n| (1..=255).contains(&n)) {
                bail!("{path}: 宏 {fq} 的 repeat {} 需要是 1..255 的整数", repr(r));
            }
        }
        if let Some(d) = raw.get("default_delay_ms") {
            if as_int(d).is_none() {
                bail!("{path}: 宏 {fq} 的 default_delay_ms 需要是整数");
            }
        }
        Ok(raw)
    }

    // ---------------- 查询 ----------------
    pub fn names(&self) -> Vec<String> {
        self.raw.keys().cloned().collect()
    }

    pub fn groups(&self) -> Vec<(String, Vec<String>)> {
        let mut out: Vec<(String, Vec<String>)> = Vec::new();
        for fq in self.raw.keys() {
            let (g, n) = fq.split_once('.').unwrap();
            match out.last_mut() {
                Some((last, names)) if last == g => names.push(n.to_string()),
                _ => out.push((g.to_string(), vec![n.to_string()])),
            }
        }
        out
    }

    fn lookup(&self, reference: &str, group: Option<&str>) -> Result<String> {
        if reference.is_empty() {
            bail!("宏引用 {reference:?} 不合法");
        }
        if reference.contains('.') {
            if self.raw.contains_key(reference) {
                return Ok(reference.to_string());
            }
        } else {
            if let Some(g) = group {
                let key = format!("{g}.{reference}"); // 同分组优先
                if self.raw.contains_key(&key) {
                    return Ok(key);
                }
            }
            let suffix = format!(".{reference}");
            let hits: Vec<&String> = self.raw.keys().filter(|fq| fq.ends_with(&suffix)).collect();
            if hits.len() == 1 {
                return Ok(hits[0].clone());
            }
            if hits.len() > 1 {
                bail!("宏引用 {reference:?} 有歧义: {}, 请写成 分组.名字", fmt_list(&hits));
            }
        }
        let close = get_close_matches(reference, self.raw.keys().map(|s| s.as_str()), 3);
        let hint = if !close.is_empty() {
            format!(", 你是不是想写: {}", fmt_list(&close))
        } else if self.raw.is_empty() {
            ", 已有的宏: (宏库是空的)".to_string()
        } else {
            format!(", 已有的宏: {}", fmt_list(&self.names()))
        };
        bail!("找不到宏 {reference:?}{hint}")
    }

    // ---------------- 展开 ----------------
    pub fn resolve(&self, reference: &str) -> Result<MacroDef> {
        let fq = self.lookup(reference, None)?;
        let (raw, src) = &self.raw[&fq];
        let steps = self.expand(&fq, &[])?;
        Ok(MacroDef {
            name: fq,
            steps,
            start: raw.get("start").and_then(|v| v.as_str()).unwrap_or("loop").to_string(),
            repeat: raw.get("repeat").and_then(as_int).unwrap_or(1),
            description: raw.get("description").map(|v| v.as_str().map(String::from).unwrap_or_else(|| repr(v))).unwrap_or_default(),
            source: src.clone(),
        })
    }

    fn expand(&self, fq: &str, stack: &[String]) -> Result<Vec<Value>> {
        if stack.iter().any(|s| s == fq) {
            let mut chain: Vec<&str> = stack.iter().map(|s| s.as_str()).collect();
            chain.push(fq);
            bail!("宏循环引用: {}", chain.join(" -> "));
        }
        let (raw, src) = &self.raw[fq];
        let group = fq.split_once('.').unwrap().0;
        let default = raw.get("default_delay_ms").and_then(as_int).unwrap_or(DEFAULT_DELAY_MS);
        let mut out: Vec<Value> = Vec::new();
        let Some(Value::Sequence(raw_steps)) = raw.get("steps") else { unreachable!("normalize 已检查 steps") };
        let mut flat = Vec::new();
        flatten(raw_steps, &mut flat);
        for (n, step) in flat.into_iter().enumerate() {
            let wher = format!("{src} 宏 {fq} 第 {} 步", n + 1);
            let Value::Mapping(step) = step else {
                bail!("{wher}: 步骤需要是字典, 实际 {}", repr(step));
            };
            if step.contains_key("use") {
                let extra = unknown_keys(step, &["use", "times"]);
                if !extra.is_empty() {
                    bail!("{wher}: use 步骤里有不认识的键 {}, 可选: use, times", fmt_keys(&extra));
                }
                let times = match step.get("times") {
                    None => 1,
                    Some(t) => match as_int(t).filter(|t| (1..=MAX_USE_TIMES).contains(t)) {
                        Some(t) => t,
                        None => bail!("{wher}: times {} 需要是 1..{MAX_USE_TIMES} 的整数", repr(&step["times"])),
                    },
                };
                let target = match &step["use"] {
                    Value::String(u) => self.lookup(u, Some(group)),
                    other => Err(crate::error::ValueError(format!("宏引用 {} 不合法", repr(other)))),
                };
                let target = match target {
                    Ok(t) => t,
                    Err(e) => bail!("{wher}: {e}"),
                };
                let mut next_stack = stack.to_vec();
                next_stack.push(fq.to_string());
                let inner = self.expand(&target, &next_stack)?;
                for _ in 0..times {
                    out.extend(inner.iter().cloned());
                }
            } else {
                let mut s = step.clone();
                if ACTION_KEYS.iter().any(|k| s.contains_key(*k)) && !s.contains_key("delay_ms") {
                    s.insert("delay_ms".into(), Value::from(default)); // 本宏的默认延迟在展开时就写死, 引用到别处也不变
                }
                out.push(Value::Mapping(s));
            }
            if out.len() > MAX_EXPANDED_STEPS {
                bail!("{fq} 展开后超过 {MAX_EXPANDED_STEPS} 步, 检查 use/times 是不是写大了");
            }
        }
        Ok(out)
    }

    /// 加载完所有文件后调用: 逐个展开并用真正的解析器检查, 配置错误在启动时就暴露而不是用到才报。
    pub fn validate_all(&self) -> Result<()> {
        for fq in self.names() {
            let d = self.resolve(&fq)?;
            if let Err(e) = parse_steps(&d.steps, DEFAULT_DELAY_MS) {
                bail!("{} 宏 {fq}: {e}", d.source);
            }
        }
        Ok(())
    }
}

impl MacroLibraryLookup for MacroLibrary {
    fn resolve_macro(&self, reference: &str) -> Result<MacroSource> {
        let d = self.resolve(reference)?;
        Ok(MacroSource { name: d.name, steps: d.steps, start: d.start, repeat: d.repeat })
    }
}

pub fn build_library(patterns: &[String], base_dir: &Path) -> Result<MacroLibrary> {
    let mut lib = MacroLibrary::new();
    lib.load_patterns(patterns, base_dir)?;
    Ok(lib)
}

// ---------------- difflib.get_close_matches 的等价实现(仅用于报错提示) ----------------

/// Ratcliff/Obershelp 相似度, 同 Python `difflib.SequenceMatcher.ratio()`(不含 autojunk 影响的长串)。
fn ratio(a: &[char], b: &[char]) -> f64 {
    fn matching(a: &[char], b: &[char]) -> usize {
        if a.is_empty() || b.is_empty() {
            return 0;
        }
        // 最长公共子串(取最靠前的)
        let (mut best, mut ai, mut bi) = (0usize, 0usize, 0usize);
        let mut prev = vec![0usize; b.len() + 1];
        for i in 0..a.len() {
            let mut cur = vec![0usize; b.len() + 1];
            for j in 0..b.len() {
                if a[i] == b[j] {
                    cur[j + 1] = prev[j] + 1;
                    if cur[j + 1] > best {
                        best = cur[j + 1];
                        ai = i + 1 - best;
                        bi = j + 1 - best;
                    }
                }
            }
            prev = cur;
        }
        if best == 0 {
            return 0;
        }
        best + matching(&a[..ai], &b[..bi]) + matching(&a[ai + best..], &b[bi + best..])
    }
    let total = a.len() + b.len();
    if total == 0 {
        return 1.0;
    }
    2.0 * matching(a, b) as f64 / total as f64
}

fn get_close_matches<'a>(word: &str, possibilities: impl Iterator<Item = &'a str>, n: usize) -> Vec<String> {
    let w: Vec<char> = word.chars().collect();
    let mut scored: Vec<(f64, String)> = possibilities
        .filter_map(|p| {
            let r = ratio(&w, &p.chars().collect::<Vec<_>>());
            (r >= 0.6).then(|| (r, p.to_string()))
        })
        .collect();
    // Python 用 heapq.nlargest 按 (score, 字符串) 比较
    scored.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap().then_with(|| y.1.cmp(&x.1)));
    scored.into_iter().take(n).map(|(_, s)| s).collect()
}
