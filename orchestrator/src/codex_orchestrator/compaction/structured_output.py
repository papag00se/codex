from __future__ import annotations

from typing import Any, Dict, Iterable, List


_STRING_LIST_FIELDS = (
    "files_touched",
    "commands_run",
    "errors",
    "accepted_fixes",
    "rejected_ideas",
    "constraints",
    "environment_assumptions",
    "pending_todos",
    "unresolved_bugs",
    "external_references",
)

_PATCH_STRING_LIST_FIELDS = (
    "add_files_touched",
    "add_commands_run",
    "add_errors",
    "add_accepted_fixes",
    "add_rejected_ideas",
    "add_constraints",
    "add_environment_assumptions",
    "add_pending_todos",
    "add_unresolved_bugs",
    "add_external_references",
)


def normalize_chunk_extraction_payload(payload: Dict[str, Any]) -> Dict[str, Any]:
    normalized = dict(payload)
    normalized["repo_state"] = _normalize_repo_state(normalized.get("repo_state"))
    for field_name in _STRING_LIST_FIELDS:
        normalized[field_name] = _normalize_string_list(normalized.get(field_name))
    normalized["test_status"] = _normalize_test_status(normalized.get("test_status"))
    normalized["latest_plan"] = _normalize_latest_plan(normalized.get("latest_plan"))
    return normalized


def normalize_merged_state_payload(payload: Dict[str, Any]) -> Dict[str, Any]:
    normalized = dict(payload)
    normalized["repo_state"] = _normalize_repo_state(normalized.get("repo_state"))
    for field_name in _STRING_LIST_FIELDS:
        normalized[field_name] = _normalize_string_list(normalized.get(field_name))
    normalized["test_status"] = _normalize_test_status(normalized.get("test_status"))
    normalized["latest_plan"] = _normalize_latest_plan(normalized.get("latest_plan"))
    return normalized


def normalize_merged_state_patch_payload(payload: Dict[str, Any]) -> Dict[str, Any]:
    normalized = dict(payload)
    normalized["objective_update"] = _stringify_scalar(normalized.get("objective_update"))
    normalized["repo_state_updates"] = _normalize_repo_state(normalized.get("repo_state_updates"))
    for field_name in _PATCH_STRING_LIST_FIELDS:
        normalized[field_name] = _normalize_string_list(normalized.get(field_name))
    normalized["add_test_status"] = _normalize_test_status(normalized.get("add_test_status"))
    normalized["latest_plan_update"] = _normalize_latest_plan(normalized.get("latest_plan_update"))
    return normalized


def _normalize_string_list(value: Any) -> List[str]:
    return _normalize_list(value, formatter=_format_default)


def _normalize_repo_state(value: Any) -> Dict[str, Any]:
    if isinstance(value, dict):
        return {str(key): item for key, item in value.items() if _stringify_scalar(key)}
    if isinstance(value, list):
        structured_entries = _normalize_repo_state_entries(value)
        if structured_entries:
            return structured_entries
        entries = _normalize_list(value, formatter=_format_default)
        return {"summary": "; ".join(entries)} if entries else {}
    text = _stringify_scalar(value)
    if not text:
        return {}
    return {"summary": text}


def chunk_extraction_response_schema() -> Dict[str, Any]:
    properties = _state_extraction_properties()
    properties["chunk_id"] = {"title": "Chunk Id", "type": "integer"}
    properties["source_token_count"] = {"title": "Source Token Count", "type": "integer"}
    ordered_properties = {
        "chunk_id": properties.pop("chunk_id"),
        **properties,
        "source_token_count": properties.pop("source_token_count"),
    }
    return _strict_object_schema("ChunkExtraction", ordered_properties)


def recent_state_response_schema() -> Dict[str, Any]:
    return _strict_object_schema("RecentStateExtraction", _state_extraction_properties())


def _state_extraction_properties() -> Dict[str, Any]:
    return {
        "objective": {"title": "Objective", "type": "string"},
        "repo_state": _repo_state_entries_schema("Repo State"),
        "files_touched": _string_list_schema("Files Touched"),
        "commands_run": _string_list_schema("Commands Run"),
        "errors": _string_list_schema("Errors"),
        "accepted_fixes": _string_list_schema("Accepted Fixes"),
        "rejected_ideas": _string_list_schema("Rejected Ideas"),
        "constraints": _string_list_schema("Constraints"),
        "environment_assumptions": _string_list_schema("Environment Assumptions"),
        "pending_todos": _string_list_schema("Pending Todos"),
        "unresolved_bugs": _string_list_schema("Unresolved Bugs"),
        "test_status": _string_list_schema("Test Status"),
        "external_references": _string_list_schema("External References"),
        "latest_plan": _string_list_schema("Latest Plan"),
    }


def merged_state_patch_response_schema() -> Dict[str, Any]:
    properties = {
        "objective_update": {"title": "Objective Update", "type": "string"},
        "repo_state_updates": _repo_state_entries_schema("Repo State Updates"),
        "add_files_touched": _string_list_schema("Add Files Touched"),
        "add_commands_run": _string_list_schema("Add Commands Run"),
        "add_errors": _string_list_schema("Add Errors"),
        "add_accepted_fixes": _string_list_schema("Add Accepted Fixes"),
        "add_rejected_ideas": _string_list_schema("Add Rejected Ideas"),
        "add_constraints": _string_list_schema("Add Constraints"),
        "add_environment_assumptions": _string_list_schema("Add Environment Assumptions"),
        "add_pending_todos": _string_list_schema("Add Pending Todos"),
        "add_unresolved_bugs": _string_list_schema("Add Unresolved Bugs"),
        "add_test_status": _string_list_schema("Add Test Status"),
        "add_external_references": _string_list_schema("Add External References"),
        "latest_plan_update": _string_list_schema("Latest Plan Update"),
    }
    return _strict_object_schema("MergedStatePatch", properties)


def _normalize_test_status(value: Any) -> List[str]:
    if isinstance(value, dict):
        return [f"{key}: {_stringify_scalar(item)}" for key, item in value.items() if _stringify_scalar(key)]
    return _normalize_list(value, formatter=_format_default)


def _normalize_latest_plan(value: Any) -> List[str]:
    return _normalize_list(value, formatter=_format_plan_step)


def _normalize_list(value: Any, *, formatter) -> List[str]:
    if value is None:
        return []
    if isinstance(value, list):
        items: Iterable[Any] = value
    else:
        items = [value]
    normalized: List[str] = []
    for item in items:
        text = formatter(item)
        if text:
            normalized.append(text)
    return normalized


def _normalize_repo_state_entries(value: List[Any]) -> Dict[str, Any]:
    normalized: Dict[str, Any] = {}
    for item in value:
        if not isinstance(item, dict):
            continue
        key = _stringify_scalar(item.get("key"))
        item_value = _stringify_scalar(item.get("value"))
        if key and item_value:
            normalized[key] = item_value
    return normalized


def _format_plan_step(item: Any) -> str:
    if isinstance(item, dict):
        step = _stringify_scalar(item.get("step"))
        status = _stringify_scalar(item.get("status"))
        if step and status:
            return f"{step} [{status}]"
        if step:
            return step
        return _format_default(item)
    return _format_default(item)


def _format_default(item: Any) -> str:
    if isinstance(item, dict):
        parts = []
        for key, value in item.items():
            key_text = _stringify_scalar(key)
            value_text = _stringify_scalar(value)
            if key_text and value_text:
                parts.append(f"{key_text}: {value_text}")
            elif value_text:
                parts.append(value_text)
        return "; ".join(parts)
    return _stringify_scalar(item)


def _stringify_scalar(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, str):
        return value.strip()
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return str(value)
    return ""


def _strict_object_schema(title: str, properties: Dict[str, Any]) -> Dict[str, Any]:
    return {
        "title": title,
        "type": "object",
        "additionalProperties": False,
        "properties": properties,
        "required": list(properties.keys()),
    }


def _repo_state_entries_schema(title: str) -> Dict[str, Any]:
    return {
        "title": title,
        "type": "array",
        "items": _strict_object_schema(
            "RepoStateEntry",
            {
                "key": {"title": "Key", "type": "string"},
                "value": {"title": "Value", "type": "string"},
            },
        ),
    }


def _string_list_schema(title: str) -> Dict[str, Any]:
    return {
        "title": title,
        "type": "array",
        "items": {"type": "string"},
    }
