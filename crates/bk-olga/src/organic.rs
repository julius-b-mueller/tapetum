//! Thin read-only view over juce_organicui's JSON layout.
//!
//! Every container is an object with optional `niceName`, `type`,
//! `parameters: [{controlAddress, value, ...}]`, `containers: {shortName: ...}`
//! and, for managers, `items: [...]`. Only parameters that differ from their
//! default are written, so every accessor here returns an `Option`.

use serde_json::Value;

#[derive(Clone, Copy)]
pub struct Node<'a>(pub &'a Value);

impl<'a> Node<'a> {
    pub fn nice_name(&self) -> Option<&'a str> {
        self.0.get("niceName").and_then(Value::as_str)
    }

    pub fn item_type(&self) -> Option<&'a str> {
        self.0.get("type").and_then(Value::as_str)
    }

    /// Value of the parameter whose control address is `/` + `short_name`.
    pub fn param(&self, short_name: &str) -> Option<&'a Value> {
        self.0
            .get("parameters")?
            .as_array()?
            .iter()
            .find(|p| {
                p.get("controlAddress")
                    .and_then(Value::as_str)
                    .and_then(|a| a.strip_prefix('/'))
                    == Some(short_name)
            })
            .and_then(|p| p.get("value"))
    }

    pub fn str_param(&self, short_name: &str) -> Option<&'a str> {
        self.param(short_name).and_then(Value::as_str)
    }

    pub fn f64_param(&self, short_name: &str) -> Option<f64> {
        self.param(short_name).and_then(Value::as_f64)
    }

    pub fn i64_param(&self, short_name: &str) -> Option<i64> {
        let v = self.param(short_name)?;
        v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64))
    }

    pub fn bool_param(&self, short_name: &str) -> Option<bool> {
        let v = self.param(short_name)?;
        v.as_bool().or_else(|| v.as_i64().map(|i| i != 0))
    }

    pub fn f32_array_param<const N: usize>(&self, short_name: &str) -> Option<[f32; N]> {
        let arr = self.param(short_name)?.as_array()?;
        if arr.len() != N {
            return None;
        }
        let mut out = [0.0; N];
        for (o, v) in out.iter_mut().zip(arr) {
            *o = v.as_f64()? as f32;
        }
        Some(out)
    }

    pub fn container(&self, short_name: &str) -> Option<Node<'a>> {
        self.0.get("containers")?.get(short_name).map(Node)
    }

    /// Follows a chain of child containers, e.g. `["artNet", "output"]`.
    pub fn container_path(&self, path: &[&str]) -> Option<Node<'a>> {
        path.iter().try_fold(*self, |n, p| n.container(p))
    }

    pub fn items(self) -> impl Iterator<Item = Node<'a>> + 'a {
        self.0
            .get("items")
            .and_then(Value::as_array)
            .map(|a| a.as_slice())
            .unwrap_or(&[])
            .iter()
            .map(Node)
    }
}
