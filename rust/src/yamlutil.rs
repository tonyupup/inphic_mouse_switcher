//! yaml 动态值的小工具: 严格取整数、Python 风格的 repr(报错信息用)、未知键检查。

use serde_yaml::Value;

/// 严格取整数: 只认 yaml 整数, 布尔和浮点不算。
pub fn as_int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

/// 宽松取整数: 整数、浮点(截断)、数字字符串都行, 布尔不行。对应 Python 的 `int(x)`。
pub fn to_int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().filter(|f| f.is_finite()).map(|f| f.trunc() as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

pub fn to_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// 字符串或数字标量 -> 字符串(yaml 里 `key: 1` 会被读成整数, 但意思是键名 "1")。
pub fn scalar_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Python 风格的 repr: 字符串带单引号, 布尔 True/False, 空值 None。
pub fn repr(v: &Value) -> String {
    match v {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("'{s}'"),
        Value::Sequence(s) => format!("[{}]", s.iter().map(repr).collect::<Vec<_>>().join(", ")),
        Value::Mapping(m) => format!(
            "{{{}}}",
            m.iter().map(|(k, v)| format!("{}: {}", repr(k), repr(v))).collect::<Vec<_>>().join(", ")
        ),
        Value::Tagged(t) => repr(&t.value),
    }
}

/// 字典里不在 `allowed` 里的键(排序后)。非字符串键用 repr 表示。
pub fn unknown_keys(map: &serde_yaml::Mapping, allowed: &[&str]) -> Vec<String> {
    let mut extra: Vec<String> = map
        .keys()
        .filter(|k| !matches!(k, Value::String(s) if allowed.contains(&s.as_str())))
        .map(|k| match k {
            Value::String(s) => s.clone(),
            other => repr(other),
        })
        .collect();
    extra.sort();
    extra
}

/// 排序后的键列表格式化成 `['a', 'b']`。
pub fn fmt_keys(keys: &[String]) -> String {
    crate::error::fmt_list(keys)
}
