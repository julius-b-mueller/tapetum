//! Tapetum's 2D plans (BK's "layouts") in a show file.
//!
//! BlinderKitten lays its 2D plans out from fixture coordinates only when it
//! imports an MVR (front, top and side, one point per fixture) and never
//! updates them. Tapetum writes the same three plans from the positions it
//! shows: "Tapetum Front" (x/z), "Tapetum Top" (x/y), "Tapetum Side" (y/z).
//! Running it again replaces them (keeping their IDs); other plans stay as
//! they are.
//!
//! Edits work on the raw JSON so everything else survives unchanged.

use serde_json::{Value, json};

use crate::short_name::to_short_name;

pub const PREFIX: &str = "Tapetum ";

/// BK's manager of 2D plans in a show file.
const KEY: &str = "2dPlans";

/// The plans: name and which two coordinates of a position they show.
const VIEWS: [(&str, usize, usize); 3] = [("Front", 0, 2), ("Top", 0, 1), ("Side", 1, 2)];

pub fn is_plan_name(name: &str) -> bool {
    name.starts_with(PREFIX)
}

/// A plan's own name (BK shows `<id> - <name>`).
fn name_of(plan: &Value) -> Option<&str> {
    plan.get("parameters")?.as_array()?.iter().find(|p| p.get("controlAddress").and_then(Value::as_str) == Some("/name"))?.get("value")?.as_str()
}

fn id_of(plan: &Value) -> Option<i64> {
    plan.get("parameters")?.as_array()?.iter().find(|p| p.get("controlAddress").and_then(Value::as_str) == Some("/id"))?.get("value")?.as_i64()
}

fn plans_mut(doc: &mut Value) -> Option<&mut Vec<Value>> {
    let manager = doc.as_object_mut()?.entry(KEY).or_insert_with(|| json!({}));
    let items = manager.as_object_mut()?.entry("items").or_insert_with(|| json!([]));
    items.as_array_mut()
}

/// BK's remote address of a plan (`/2dPlans/7_TapetumFront`).
pub fn remote_path(id: i64, name: &str) -> String {
    format!("/{KEY}/{}", to_short_name(&format!("{id} - {name}"), false))
}

/// Writes (or replaces) Tapetum's plans for these fixtures (ID, position in
/// meters); returns each plan's ID and name.
pub fn set_plans(doc: &mut Value, fixtures: &[(i64, [f32; 3])]) -> Result<Vec<(i64, String)>, String> {
    let plans = plans_mut(doc).ok_or("the show file has no place for 2D plans")?;
    let mut next = plans.iter().filter_map(id_of).max().unwrap_or(0) + 1;
    let mut written = Vec::new();
    for (view, a, b) in VIEWS {
        let name = format!("{PREFIX}{view}");
        let existing = plans.iter().position(|p| name_of(p) == Some(&name));
        let id = match existing.and_then(|i| id_of(&plans[i])) {
            Some(id) => id,
            None => {
                next += 1;
                next - 1
            }
        };
        let plan = plan(id, &name, fixtures, a, b);
        match existing {
            Some(i) => plans[i] = plan,
            None => plans.push(plan),
        }
        written.push((id, name));
    }
    Ok(written)
}

/// Whether the show's Tapetum plans show these fixtures where they are now:
/// `None` without Tapetum plans.
pub fn up_to_date(plans: &[crate::model::Plan], fixtures: &[(i64, [f32; 3])]) -> Option<bool> {
    if !plans.iter().any(|p| is_plan_name(&p.name)) {
        return None;
    }
    let round = |v: f32| (v * 1000.0).round() as i64;
    Some(VIEWS.iter().all(|(view, a, b)| {
        let name = format!("{PREFIX}{view}");
        let Some(plan) = plans.iter().find(|p| p.name == name) else { return false };
        let mut have: Vec<(i64, i64, i64)> = plan.points.iter().map(|(id, p)| (*id, round(p[0]), round(p[1]))).collect();
        let mut want: Vec<(i64, i64, i64)> = fixtures.iter().map(|(id, p)| (*id, round(p[*a]), round(p[*b]))).collect();
        have.sort_unstable();
        want.sort_unstable();
        have == want
    }))
}

/// Removes Tapetum's plans; returns their IDs and names.
pub fn remove_plans(doc: &mut Value) -> Vec<(i64, String)> {
    let Some(plans) = plans_mut(doc) else { return Vec::new() };
    let removed: Vec<(i64, String)> = plans.iter().filter_map(|p| Some((id_of(p)?, name_of(p).filter(|n| is_plan_name(n))?.to_string()))).collect();
    plans.retain(|p| !name_of(p).is_some_and(is_plan_name));
    removed
}

fn plan(id: i64, name: &str, fixtures: &[(i64, [f32; 3])], a: usize, b: usize) -> Value {
    let round = |v: f32| (f64::from(v) * 1000.0).round() / 1000.0;
    let points: Vec<[f64; 2]> = fixtures.iter().map(|(_, p)| [round(p[a]), round(p[b])]).collect();
    // As BK's "fit to content": the points with a meter around them.
    let range = |k: usize| -> [f64; 2] {
        let (lo, hi) = points.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p[k]), hi.max(p[k])));
        if lo > hi { [-5.0, 5.0] } else { [lo - 1.0, hi + 1.0] }
    };
    let paths: Vec<Value> = fixtures
        .iter()
        .zip(&points)
        .map(|((fid, _), p)| {
            json!({
                "niceName": format!("Fixture {fid}"),
                "type": "BKPath",
                "parameters": [{ "value": p, "controlAddress": "/position" }],
                "containers": { "selections": { "items": [{
                    "niceName": "CommandSelection",
                    "type": "BaseItem",
                    "parameters": [
                        { "value": "Fixture", "controlAddress": "/targetType" },
                        { "value": fid, "hexMode": false, "controlAddress": "/id" },
                    ],
                }] } },
            })
        })
        .collect();
    json!({
        "niceName": format!("{id} - {name}"),
        "type": "Layout",
        "parameters": [
            { "value": id, "hexMode": false, "controlAddress": "/id" },
            { "value": name, "controlAddress": "/name" },
            { "value": range(0), "controlAddress": "/fromToX" },
            { "value": range(1), "controlAddress": "/fromToY" },
        ],
        "containers": { "paths": { "items": paths } },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_are_added_replaced_and_removed() {
        let mut doc = json!({ "2dPlans": { "items": [{ "niceName": "3 - Mine", "parameters": [{ "value": 3, "controlAddress": "/id" }, { "value": "Mine", "controlAddress": "/name" }] }] } });
        let fixtures = [(11, [-4.8, 1.8, 6.65]), (12, [1.6, 1.8, 6.65])];
        let first = set_plans(&mut doc, &fixtures).unwrap();
        assert_eq!(first, vec![(4, "Tapetum Front".to_string()), (5, "Tapetum Top".to_string()), (6, "Tapetum Side".to_string())]);
        let plans = doc["2dPlans"]["items"].as_array().unwrap();
        assert_eq!(plans.len(), 4);
        // Front: x and z.
        assert_eq!(plans[1]["containers"]["paths"]["items"][0]["parameters"][0]["value"], json!([-4.8, 6.65]));
        assert_eq!(plans[1]["parameters"][2]["value"], json!([-5.8, 2.6]));

        // Again, with a fixture moved: the same IDs, nothing doubled.
        let again = set_plans(&mut doc, &[(11, [0.0, 0.0, 5.0])]).unwrap();
        assert_eq!(again, first);
        let plans = doc["2dPlans"]["items"].as_array().unwrap();
        assert_eq!(plans.len(), 4);
        assert_eq!(plans[3]["containers"]["paths"]["items"][0]["parameters"][0]["value"], json!([0.0, 5.0]));

        assert_eq!(remove_plans(&mut doc).len(), 3);
        assert_eq!(doc["2dPlans"]["items"].as_array().unwrap().len(), 1, "the user's own plan stays");
        assert_eq!(remote_path(4, "Tapetum Front"), "/2dPlans/4_TapetumFront");
    }

    #[test]
    fn plans_know_when_they_are_behind() {
        let fixtures = [(11, [-4.8, 1.8, 6.65]), (12, [1.6, 1.8, 6.65])];
        let mut doc = json!({ "2dPlans": { "items": [] } });
        let show = |doc: &Value| crate::model::Show::from_json(doc).plans;
        assert_eq!(up_to_date(&show(&doc), &fixtures), None);
        set_plans(&mut doc, &fixtures).unwrap();
        assert_eq!(up_to_date(&show(&doc), &fixtures), Some(true));
        assert_eq!(up_to_date(&show(&doc), &[(11, [-4.8, 1.8, 6.65]), (12, [1.6, 2.0, 6.65])]), Some(false), "moved");
        assert_eq!(up_to_date(&show(&doc), &fixtures[..1]), Some(false), "one fixture less");
    }

    #[test]
    fn a_show_without_plans_gets_them() {
        let mut doc = json!({ "2dPlans": { "viewOffset": [0, 0], "viewZoom": 1.0 } });
        assert_eq!(set_plans(&mut doc, &[(1, [0.0, 0.0, 0.0])]).unwrap()[0], (1, "Tapetum Front".to_string()));
    }
}
