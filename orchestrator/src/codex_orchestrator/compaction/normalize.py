from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional

from ..schemas.config import settings
from ..routing.metrics import estimate_tokens

_TEXT_BLOCK_TYPES = {'text', 'input_text', 'output_text'}
_STRUCTURED_BLOCK_TYPES = {'tool_use', 'tool_result', 'function_call', 'function_call_output'}
_ATTACHMENT_BLOCK_TYPES = {'image', 'input_image', 'localImage', 'local_image', 'file', 'input_file'}
_CODEX_BOOTSTRAP_PREFIX = 'You are Codex'


@dataclass
class NormalizedTranscript:
    compactable_items: List[Dict[str, Any]] = field(default_factory=list)
    precompacted_items: List[Dict[str, Any]] = field(default_factory=list)
    preserved_tail: List[Dict[str, Any]] = field(default_factory=list)


def sanitize_inline_compaction_payload(payload: Dict[str, Any]) -> Dict[str, Any]:
    sanitized = dict(payload)
    instructions = sanitized.get('instructions')
    if _is_codex_bootstrap_instructions(instructions):
        sanitized.pop('instructions', None)
    return sanitized


def normalize_transcript_for_compaction(
    items: List[Dict[str, Any]],
    *,
    max_item_tokens: int,
) -> NormalizedTranscript:
    raw_items = [_annotate_compaction_index(_sanitize_raw_item(item), index) for index, item in enumerate(items)]
    latest_index = next((index for index in range(len(raw_items) - 1, -1, -1) if raw_items[index] is not None), None)

    normalized = NormalizedTranscript()
    if latest_index is not None and raw_items[latest_index] is not None:
        normalized.preserved_tail.append(raw_items[latest_index])

    for index, raw_item in enumerate(raw_items):
        if index == latest_index:
            continue
        precompacted, remaining = _split_precompacted_item(raw_item)
        if precompacted is not None:
            normalized.precompacted_items.append(precompacted)
        compactable = _sanitize_compactable_item_from_raw(remaining, max_item_tokens=max_item_tokens)
        if compactable is not None:
            normalized.compactable_items.append(compactable)
    return normalized


def _sanitize_raw_item(item: Any) -> Optional[Dict[str, Any]]:
    if not isinstance(item, dict):
        return None
    sanitized = _strip_encrypted_content(item)
    if not isinstance(sanitized, dict) or not sanitized:
        return None
    if 'content' not in sanitized:
        return sanitized
    content = _sanitize_raw_content(sanitized.get('content'))
    if content is None:
        return None
    sanitized['content'] = content
    return sanitized


def _sanitize_compactable_item(item: Any, *, max_item_tokens: int) -> Optional[Dict[str, Any]]:
    raw_item = _sanitize_raw_item(item)
    return _sanitize_compactable_item_from_raw(raw_item, max_item_tokens=max_item_tokens)


def _sanitize_compactable_item_from_raw(raw_item: Any, *, max_item_tokens: int) -> Optional[Dict[str, Any]]:
    if raw_item is None:
        return None
    if 'content' not in raw_item:
        return raw_item if estimate_tokens(raw_item) <= max_item_tokens else None

    content = raw_item.get('content')
    if isinstance(content, str):
        return raw_item if estimate_tokens(content) <= max_item_tokens else None
    if not isinstance(content, list):
        return raw_item if estimate_tokens(raw_item) <= max_item_tokens else None

    blocks: List[Dict[str, Any]] = []
    for block in content:
        compactable_block = _sanitize_compactable_block(block, max_item_tokens=max_item_tokens)
        if compactable_block is not None:
            blocks.append(compactable_block)
    if not blocks:
        return None
    compactable_item = dict(raw_item)
    compactable_item['content'] = blocks
    return compactable_item


def _annotate_compaction_index(item: Optional[Dict[str, Any]], index: int) -> Optional[Dict[str, Any]]:
    if item is None:
        return None
    annotated = dict(item)
    annotated['_compaction_index'] = index
    return annotated


def _split_precompacted_item(raw_item: Any) -> tuple[Optional[Dict[str, Any]], Optional[Dict[str, Any]]]:
    if raw_item is None:
        return None, None
    content = raw_item.get('content')
    if isinstance(content, str):
        if _is_precompacted_summary_text(content):
            return raw_item, None
        return None, raw_item
    if not isinstance(content, list):
        return None, raw_item

    precompacted_blocks: List[Dict[str, Any]] = []
    remaining_blocks: List[Dict[str, Any]] = []
    for block in content:
        if _is_precompacted_summary_block(block):
            precompacted_blocks.append(block)
        else:
            remaining_blocks.append(block)

    precompacted_item = None
    if precompacted_blocks:
        precompacted_item = dict(raw_item)
        precompacted_item['content'] = precompacted_blocks

    remaining_item = None
    if remaining_blocks:
        remaining_item = dict(raw_item)
        remaining_item['content'] = remaining_blocks
    return precompacted_item, remaining_item


def _sanitize_raw_content(content: Any) -> Optional[Any]:
    if isinstance(content, str):
        return content if content else None
    if not isinstance(content, list):
        return content if content is not None else None

    blocks: List[Dict[str, Any]] = []
    for block in content:
        sanitized = _sanitize_raw_block(block)
        if sanitized is not None:
            blocks.append(sanitized)
    return blocks or None


def _sanitize_raw_block(block: Any) -> Optional[Dict[str, Any]]:
    if not isinstance(block, dict):
        return None
    sanitized = _strip_encrypted_content(block)
    if not isinstance(sanitized, dict) or not sanitized:
        return None
    block_type = sanitized.get('type')
    if block_type in _TEXT_BLOCK_TYPES:
        text = sanitized.get('text') or sanitized.get('input_text') or sanitized.get('output_text') or ''
        return sanitized if text else None
    return sanitized


def _sanitize_compactable_block(block: Any, *, max_item_tokens: int) -> Optional[Dict[str, Any]]:
    raw_block = _sanitize_raw_block(block)
    if raw_block is None:
        return None
    block_type = raw_block.get('type')
    if block_type in _ATTACHMENT_BLOCK_TYPES:
        return None
    if block_type in _TEXT_BLOCK_TYPES:
        text = raw_block.get('text') or raw_block.get('input_text') or raw_block.get('output_text') or ''
        return raw_block if estimate_tokens(text) <= max_item_tokens else None
    if block_type in {'tool_result', 'function_call_output'}:
        return None
    if block_type in _STRUCTURED_BLOCK_TYPES:
        return raw_block if estimate_tokens(raw_block) <= max_item_tokens else None
    return None


def _is_precompacted_summary_block(block: Dict[str, Any]) -> bool:
    block_type = block.get('type')
    if block_type not in _TEXT_BLOCK_TYPES:
        return False
    text = block.get('text') or block.get('input_text') or block.get('output_text') or ''
    return _is_precompacted_summary_text(text)


def _is_precompacted_summary_text(text: Any) -> bool:
    if not isinstance(text, str):
        return False
    normalized = text.strip()
    if not normalized:
        return False
    if normalized.startswith('Another language model started to solve this problem and produced a summary of its thinking process.'):
        return True
    return '## Thread Summary for Continuation' in normalized and 'Latest Real User Intent' in normalized


def _strip_encrypted_content(value: Any) -> Any:
    if isinstance(value, dict):
        sanitized: Dict[str, Any] = {}
        for key, child in value.items():
            if key == 'encrypted_content':
                continue
            normalized = _strip_encrypted_content(child)
            if normalized is None:
                continue
            if isinstance(normalized, (dict, list)) and not normalized:
                continue
            sanitized[key] = normalized
        return sanitized
    if isinstance(value, list):
        sanitized_list = []
        for child in value:
            normalized = _strip_encrypted_content(child)
            if normalized is None:
                continue
            if isinstance(normalized, (dict, list)) and not normalized:
                continue
            sanitized_list.append(normalized)
        return sanitized_list
    return value


def _is_codex_bootstrap_instructions(value: Any) -> bool:
    if not isinstance(value, str):
        return False
    text = value.strip()
    sentinel = settings.inline_compact_sentinel
    if sentinel and text.startswith(sentinel):
        text = text[len(sentinel):].strip()
    return text.startswith(_CODEX_BOOTSTRAP_PREFIX)
