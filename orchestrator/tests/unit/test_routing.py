"""Tests for routing metrics and tool adapter — verifying migrated behavior."""
from __future__ import annotations

from codex_orchestrator.routing.metrics import (
    estimate_tokens,
    extract_task_metrics,
    FILE_REFERENCE_PATTERN,
    ERROR_LINE_PATTERN,
    FAILURE_STATUSES,
)
from codex_orchestrator.providers.tool_adapter import (
    recover_ollama_message,
    recover_stream_ollama_message,
    normalize_ollama_tools,
    anthropic_messages_to_ollama,
    ollama_message_to_anthropic_content,
    is_devstral_model,
)


class TestEstimateTokens:
    def test_empty_string(self):
        assert estimate_tokens("") == 0

    def test_none(self):
        assert estimate_tokens(None) == 0

    def test_short_string(self):
        # "hello" = 5 chars → (5+3)//4 = 2
        assert estimate_tokens("hello") == 2

    def test_longer_string(self):
        text = "a" * 100
        assert estimate_tokens(text) == 25  # (100+3)//4 = 25

    def test_dict_input(self):
        # Dicts get JSON-stringified first
        result = estimate_tokens({"key": "value"})
        assert result > 0

    def test_list_input(self):
        result = estimate_tokens([1, 2, 3])
        assert result > 0


class TestExtractTaskMetrics:
    def test_basic_prompt(self):
        metrics = extract_task_metrics(prompt="Fix the bug in auth.py")
        assert metrics["user_prompt_chars"] == 22
        assert metrics["user_prompt_lines"] == 1
        assert metrics["user_prompt_tokens"] > 0
        assert metrics["file_reference_count"] >= 1  # auth.py

    def test_all_27_metrics_present(self):
        """Verify all 27 metrics from the routing logic reference are extracted."""
        metrics = extract_task_metrics(
            prompt="Fix the bug in auth.py",
            trajectory=[{"role": "user", "content": "previous message"}],
            metadata={"key": "value"},
        )
        expected_keys = {
            "user_prompt_chars", "user_prompt_lines", "user_prompt_tokens",
            "trajectory_chars", "trajectory_lines", "trajectory_tokens",
            "message_count", "user_message_count", "assistant_message_count",
            "tool_message_count", "tool_call_count", "command_count",
            "command_output_tokens", "file_reference_count", "unique_file_reference_count",
            "code_block_count", "json_block_count", "diff_line_count",
            "error_line_count", "stack_trace_count", "prior_failure_count",
            "question_count", "metadata_key_count",
        }
        assert expected_keys.issubset(set(metrics.keys()))

    def test_error_detection(self):
        metrics = extract_task_metrics(
            prompt="There was an error in the build",
            trajectory=[{"role": "assistant", "content": "Traceback (most recent call last):\n  File 'test.py'"}],
        )
        assert metrics["error_line_count"] >= 1
        assert metrics["stack_trace_count"] >= 1

    def test_file_references(self):
        metrics = extract_task_metrics(
            prompt="Update src/auth.py and tests/test_auth.py"
        )
        assert metrics["file_reference_count"] >= 2
        assert metrics["unique_file_reference_count"] >= 2

    def test_command_detection(self):
        metrics = extract_task_metrics(
            prompt="$ npm install\n$ pytest tests/",
        )
        assert metrics["command_count"] >= 2

    def test_question_count(self):
        metrics = extract_task_metrics(prompt="What is wrong? Can you fix it?")
        assert metrics["question_count"] == 2

    def test_empty_prompt(self):
        metrics = extract_task_metrics(prompt="")
        assert metrics["user_prompt_chars"] == 0
        assert metrics["user_prompt_tokens"] == 0

    def test_prior_failure_count(self):
        metrics = extract_task_metrics(
            prompt="retry",
            trajectory={
                "attempts": [
                    {"status": "error"},
                    {"status": "success"},
                    {"status": "timeout"},
                ]
            },
        )
        assert metrics["prior_failure_count"] == 2  # error + timeout


class TestFileReferencePattern:
    def test_python_file(self):
        assert FILE_REFERENCE_PATTERN.search("update auth.py")

    def test_nested_path(self):
        assert FILE_REFERENCE_PATTERN.search("edit src/utils/helpers.ts")

    def test_with_line_number(self):
        assert FILE_REFERENCE_PATTERN.search("error in auth.py:42")


class TestFailureStatuses:
    def test_known_failures(self):
        assert "error" in FAILURE_STATUSES
        assert "failed" in FAILURE_STATUSES
        assert "timeout" in FAILURE_STATUSES
        assert "timed_out" in FAILURE_STATUSES
        assert "cancelled" in FAILURE_STATUSES
        assert "low_confidence" in FAILURE_STATUSES

    def test_success_not_failure(self):
        assert "success" not in FAILURE_STATUSES


class TestRecoverOllamaMessage:
    def test_passthrough_plain_text(self):
        msg = {"content": "Hello, just text", "role": "assistant"}
        assert recover_ollama_message(msg) == msg

    def test_passthrough_existing_tool_calls(self):
        msg = {"content": "text", "tool_calls": [{"function": {"name": "test", "arguments": {}}}]}
        assert recover_ollama_message(msg) == msg

    def test_recover_embedded_tool_use(self):
        msg = {"content": 'Some text\n\n{"type": "tool_use", "name": "exec_command", "input": {"cmd": "ls"}}\n\nMore text'}
        result = recover_ollama_message(msg)
        assert "tool_calls" in result
        assert result["tool_calls"][0]["function"]["name"] == "exec_command"
        assert result["tool_calls"][0]["function"]["arguments"] == {"cmd": "ls"}
        assert "Some text" in result["content"]
        assert "More text" in result["content"]

    def test_recover_json_blob_with_tool_calls(self):
        msg = {"content": '{"content": "thinking...", "tool_calls": [{"function": {"name": "shell", "arguments": {"cmd": "pwd"}}}]}'}
        result = recover_ollama_message(msg)
        assert result["tool_calls"][0]["function"]["name"] == "shell"
        assert result["content"] == "thinking..."

    def test_recover_fenced_json(self):
        msg = {"content": '```json\n{"type": "tool_use", "name": "read_file", "input": {"path": "test.py"}}\n```'}
        result = recover_ollama_message(msg)
        assert "tool_calls" in result
        assert result["tool_calls"][0]["function"]["name"] == "read_file"

    def test_tool_result_blocks_stripped_with_tool_use(self):
        # tool_result blocks are stripped when a tool_use is also present
        msg = {"content": 'text\n\n{"type": "tool_use", "name": "shell", "input": {"cmd": "ls"}}\n\n{"type": "tool_result", "content": "output"}\n\nmore text'}
        result = recover_ollama_message(msg)
        assert "tool_calls" in result
        assert "tool_result" not in result.get("content", "")

    def test_arguments_string_parsed(self):
        msg = {"content": '{"content": "", "tool_calls": [{"function": {"name": "test", "arguments": "{\\"key\\": \\"value\\"}"}}]}'}
        result = recover_ollama_message(msg)
        assert result["tool_calls"][0]["function"]["arguments"] == {"key": "value"}


class TestStreamRecovery:
    def test_partial_tool_block_dropped(self):
        msg = {"content": 'text\n\n{"type": "tool_use", "name": "incomplete'}
        result = recover_stream_ollama_message(msg)
        assert "tool_calls" not in result or not result.get("tool_calls")


class TestNormalizeOllamaTools:
    def test_anthropic_format(self):
        tools = [{"name": "shell", "description": "Run command", "input_schema": {"type": "object"}}]
        result = normalize_ollama_tools(tools)
        assert result[0]["type"] == "function"
        assert result[0]["function"]["name"] == "shell"

    def test_openai_format_passthrough(self):
        tools = [{"type": "function", "function": {"name": "test", "parameters": {}}}]
        result = normalize_ollama_tools(tools)
        assert result == tools

    def test_none_input(self):
        assert normalize_ollama_tools(None) is None


class TestAnthropicToOllama:
    def test_simple_message(self):

        class Msg:
            def __init__(self, role, content):
                self.role = role
                self.content = content

        messages = [Msg("user", "hello"), Msg("assistant", "hi")]
        result = anthropic_messages_to_ollama(messages)
        assert len(result) == 2
        assert result[0]["role"] == "user"
        assert result[0]["content"] == "hello"

    def test_tool_use_conversion(self):

        class Msg:
            def __init__(self, role, content):
                self.role = role
                self.content = content

        messages = [
            Msg("assistant", [
                {"type": "text", "text": "Let me run that"},
                {"type": "tool_use", "id": "call_1", "name": "shell", "input": {"cmd": "ls"}},
            ])
        ]
        result = anthropic_messages_to_ollama(messages)
        assert result[0]["role"] == "assistant"
        assert result[0]["tool_calls"][0]["function"]["name"] == "shell"


class TestIsDevstralModel:
    def test_devstral_detected(self):
        assert is_devstral_model("devstral-v0.1")
        assert is_devstral_model("Devstral-Large")

    def test_non_devstral(self):
        assert not is_devstral_model("qwen3-coder:30b")
        assert not is_devstral_model("gpt-5.4")
