//! So hợp đồng tool MCP (`docs/mcp-tools.json`) hai bản.

use super::*;

/// So hai manifest tool MCP (`docs/mcp-tools.json`, D-233). Tool ghép theo tên; schema đầu vào so
/// theo chiều request, schema kết quả theo chiều response, bằng cùng bộ so của OpenAPI.
pub fn diff_tools(old: &Value, new: &Value, today: NaiveDate) -> Report {
    let mut report = Report::default();
    let index = |manifest: &Value| -> BTreeMap<String, Value> {
        manifest
            .get("tools")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tool| Some((tool.get("name")?.as_str()?.to_owned(), tool.clone())))
            .collect()
    };
    let (old_tools, new_tools) = (index(old), index(new));
    for (name, old_tool) in &old_tools {
        let Some(new_tool) = new_tools.get(name) else {
            let sunset = old_tool
                .pointer("/deprecation/sunset")
                .and_then(Value::as_str)
                .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok());
            match sunset {
                Some(sunset) if sunset <= today => report.push(
                    Level::Note,
                    "",
                    name.clone(),
                    format!("tool removed after its sunset date {sunset}"),
                ),
                _ => report.breaking_at(
                    "tool.removed",
                    name.clone(),
                    "This tool of the base version is gone",
                ),
            }
            continue;
        };
        let flag = |tool: &Value, key: &str| {
            tool.pointer(&format!("/annotations/{key}")).and_then(Value::as_bool) == Some(true)
        };
        if flag(old_tool, "read_only") && !flag(new_tool, "read_only") {
            report.breaking_at(
                "tool.no_longer_read_only",
                name.clone(),
                "The tool is no longer read-only",
            );
        }
        if !flag(old_tool, "requires_approval_for_agent")
            && flag(new_tool, "requires_approval_for_agent")
        {
            report.breaking_at(
                "tool.approval_added",
                name.clone(),
                "Agents now need an approval to call it",
            );
        }
        if !flag(old_tool, "destructive") && flag(new_tool, "destructive") {
            report.push(
                Level::Warning,
                "tool.now_destructive",
                name.clone(),
                "clients may now ask the user before calling it",
            );
        }
        if old_tool.get("permissions") != new_tool.get("permissions") {
            report.push(
                Level::Warning,
                "tool.permissions_changed",
                name.clone(),
                "agents with an old key may no longer see the tool; review before releasing",
            );
        }
        let mut inner = Report::default();
        let mut compare =
            Comparison { old_doc: old, new_doc: new, report: &mut inner, visited: HashSet::new() };
        if let (Some(old_input), Some(new_input)) =
            (old_tool.get("input_schema"), new_tool.get("input_schema"))
        {
            compare.schema(&format!("{name} $"), old_input, new_input, Direction::Request, 0);
        }
        match (old_tool.get("output_schema"), new_tool.get("output_schema")) {
            (Some(old_output), Some(new_output))
                if !old_output.is_null() && !new_output.is_null() =>
            {
                compare.schema(
                    &format!("{name} result $"),
                    old_output,
                    new_output,
                    Direction::Response,
                    0,
                );
            }
            (Some(old_output), new_output)
                if !old_output.is_null() && new_output.is_none_or(Value::is_null) =>
            {
                inner.breaking_at(
                    "tool.result_changed",
                    format!("{name} result"),
                    "The result schema is gone",
                );
            }
            _ => {}
        }
        for finding in inner.findings {
            let rule = match (finding.level, finding.rule) {
                (Level::Breaking, "request.property_required")
                    if finding.message.starts_with("A new") =>
                {
                    "tool.parameter_added_required"
                }
                (Level::Breaking, "request.property_required") => "tool.parameter_became_required",
                (Level::Breaking, "request.property_removed") => "tool.parameter_removed",
                (Level::Breaking, "request.type_narrowed" | "schema.composite_changed")
                    if !finding.at.contains(" result ") =>
                {
                    "tool.parameter_type_changed"
                }
                (Level::Breaking, "request.constraint_tightened" | "enum.value_removed")
                    if !finding.at.contains(" result ") =>
                {
                    "tool.constraint_tightened"
                }
                (Level::Breaking, "response.property_removed") => "tool.result_property_removed",
                (Level::Breaking, _) => "tool.result_changed",
                (_, rule) => rule,
            };
            report.push(finding.level, rule, finding.at, finding.message);
        }
    }
    for name in new_tools.keys().filter(|name| !old_tools.contains_key(*name)) {
        report.push(Level::Note, "", name.clone(), "new tool");
    }
    report
}
