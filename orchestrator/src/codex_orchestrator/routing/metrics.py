from __future__ import annotations

import json
import re
from typing import Any, Dict, Iterable, List, Mapping

import tiktoken


FILE_EXTENSIONS = (
    'py',
    'js',
    'ts',
    'tsx',
    'jsx',
    'md',
    'yml',
    'yaml',
    'json',
    'toml',
    'go',
    'java',
    'rb',
    'php',
    'rs',
    'cpp',
    'c',
    'h',
    'sql',
    'sh',
    'bash',
    'html',
    'css',
    'scss',
    'vue',
    'svelte',
    'kt',
    'kts',
)
FILE_EXTENSION_PATTERN = '|'.join(FILE_EXTENSIONS)

FILE_REFERENCE_PATTERN = re.compile(
    rf'(?<!\w)(?:`)?(?P<path>(?:[A-Za-z0-9._/\-]+/)*[A-Za-z0-9._-]+\.(?:{FILE_EXTENSION_PATTERN})(?:[:#]\d+)?)(?:`)?',
    re.IGNORECASE,
)
ERROR_LINE_PATTERN = re.compile(r'(?im)^.*(?:error|exception|failed|failure|traceback|panic|fatal).*$')
STACK_TRACE_PATTERN = re.compile(r'(?im)(?:traceback \(most recent call last\)|\bat [^\n]+:\d+|file "[^"\n]+", line \d+)')
JSON_BLOCK_PATTERN = re.compile(r'```(?:json|javascript)\b', re.IGNORECASE)
DIFF_LINE_PATTERN = re.compile(r'(?m)^(?:\+|-)(?!\+\+\+|---).+$')
FENCED_BLOCK_PATTERN = re.compile(r'```')
TOOL_CALL_PATTERN = re.compile(r'(?i)(?:tool_call|tool_calls|function_call|recipient_name)')
COMMAND_LINE_PATTERN = re.compile(r'(?m)^(?:\$ |(?:bash|sh|zsh|fish|python|python3|node|npm|pnpm|yarn|uv|pytest|git|rg|sed|cat|ls|curl|ollama)\b)')
QUESTION_PATTERN = re.compile(r'\?')

FAILURE_STATUSES = {
    'error',
    'failed',
    'failure',
    'timeout',
    'timed_out',
    'cancelled',
    'canceled',
    'low_confidence',
    'malformed_output',
}


def estimate_tokens(value: Any) -> int:
    text = _stringify(value)
    if not text:
        return 0
    return max(1, (len(text) + 3) // 4)


def estimate_model_tokens(value: Any, *, model: str) -> int:
    text = _stringify(value)
    if not text:
        return 0
    encoding = _encoding_for_openai_model(model)
    return len(encoding.encode(text))


def estimate_openai_tokens(value: Any, *, model: str) -> int:
    return estimate_model_tokens(value, model=model)


def extract_task_metrics(
    prompt: str,
    trajectory: Any = None,
    metadata: Mapping[str, Any] | None = None,
) -> Dict[str, int]:
    user_prompt = prompt or ''
    trajectory_value = trajectory if trajectory is not None else []
    trajectory_text = _stringify(trajectory_value)
    combined_text = '\n\n'.join(part for part in (user_prompt, trajectory_text) if part)
    messages = _message_like_items(trajectory_value)
    file_references = [match.group('path') for match in FILE_REFERENCE_PATTERN.finditer(combined_text)]
    unique_file_references = {path.lower() for path in file_references}
    attempts = _attempt_items(trajectory_value)

    return {
        'user_prompt_chars': len(user_prompt),
        'user_prompt_lines': _line_count(user_prompt),
        'user_prompt_tokens': estimate_tokens(user_prompt),
        'trajectory_chars': len(trajectory_text),
        'trajectory_lines': _line_count(trajectory_text),
        'trajectory_tokens': estimate_tokens(trajectory_text),
        'message_count': len(messages),
        'user_message_count': sum(1 for item in messages if item.get('role') == 'user'),
        'assistant_message_count': sum(1 for item in messages if item.get('role') == 'assistant'),
        'tool_message_count': sum(1 for item in messages if item.get('role') in {'tool', 'function'}),
        'tool_call_count': len(TOOL_CALL_PATTERN.findall(combined_text)),
        'command_count': len(COMMAND_LINE_PATTERN.findall(combined_text)),
        'command_output_tokens': estimate_tokens(_command_output_text(trajectory_value)),
        'file_reference_count': len(file_references),
        'unique_file_reference_count': len(unique_file_references),
        'code_block_count': len(FENCED_BLOCK_PATTERN.findall(combined_text)) // 2,
        'json_block_count': len(JSON_BLOCK_PATTERN.findall(combined_text)),
        'diff_line_count': len(DIFF_LINE_PATTERN.findall(combined_text)),
        'error_line_count': len(ERROR_LINE_PATTERN.findall(combined_text)),
        'stack_trace_count': len(STACK_TRACE_PATTERN.findall(combined_text)),
        'prior_failure_count': sum(1 for item in attempts if _status(item) in FAILURE_STATUSES),
        'question_count': len(QUESTION_PATTERN.findall(user_prompt)),
        'metadata_key_count': len(metadata or {}),
    }


def _stringify(value: Any) -> str:
    if value is None:
        return ''
    if isinstance(value, str):
        return value
    try:
        return json.dumps(value, ensure_ascii=False, separators=(',', ':'))
    except TypeError:
        return str(value)


def _encoding_for_openai_model(model: str):
    normalized = (model or '').strip().lower()
    if normalized.startswith('gpt-5') or 'codex' in normalized:
        return tiktoken.get_encoding('o200k_base')
    try:
        return tiktoken.encoding_for_model(model)
    except KeyError:
        return tiktoken.get_encoding('o200k_base')


def _line_count(text: str) -> int:
    if not text:
        return 0
    return text.count('\n') + 1


def _message_like_items(trajectory: Any) -> List[Dict[str, Any]]:
    if isinstance(trajectory, list):
        return [item for item in trajectory if isinstance(item, dict) and 'role' in item]
    if isinstance(trajectory, dict):
        messages = trajectory.get('messages')
        if isinstance(messages, list):
            return [item for item in messages if isinstance(item, dict) and 'role' in item]
    return []


def _attempt_items(trajectory: Any) -> Iterable[Dict[str, Any]]:
    if isinstance(trajectory, dict):
        attempts = trajectory.get('attempts')
        if isinstance(attempts, list):
            return [item for item in attempts if isinstance(item, dict)]
    return []


def _status(item: Mapping[str, Any]) -> str:
    value = item.get('status')
    return str(value).strip().lower() if value is not None else ''


def _command_output_text(trajectory: Any) -> str:
    parts: List[str] = []
    for item in _iter_dicts(trajectory):
        for key in ('stdout', 'stderr', 'output', 'result'):
            value = item.get(key)
            if isinstance(value, str) and value:
                parts.append(value)
    return '\n'.join(parts)


def _iter_dicts(value: Any) -> Iterable[Mapping[str, Any]]:
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from _iter_dicts(child)
        return
    if isinstance(value, list):
        for child in value:
            yield from _iter_dicts(child)
