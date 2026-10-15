import os
import pytest
from openai import OpenAI


@pytest.fixture(scope="session")
def base_url():
    return os.environ.get("OPENAI_BASE_URL", "http://127.0.0.1:7480/v1")


@pytest.fixture(scope="session")
def api_key():
    return os.environ.get("OPENAI_API_KEY", "test-key")


@pytest.fixture(scope="session")
def client(base_url, api_key):
    return OpenAI(base_url=base_url, api_key=api_key)


@pytest.fixture(scope="session")
def model_id(client):
    models = client.models.list()
    assert len(models.data) > 0, "Expected at least one model registered on server"
    return models.data[0].id


def test_models_list(client):
    models = client.models.list()
    assert models.object == "list"
    assert len(models.data) > 0, "Model list must be non-empty"
    first = models.data[0]
    assert first.id
    assert first.object == "model"
    assert first.owned_by == "pocketai"


def test_chat_completion_non_stream(client, model_id):
    completion = client.chat.completions.create(
        model=model_id,
        messages=[{"role": "user", "content": "Hello PocketAI"}],
        stream=False,
    )
    assert completion.object == "chat.completion"
    assert completion.model == model_id
    assert len(completion.choices) == 1
    choice = completion.choices[0]
    assert choice.finish_reason == "stop"
    assert choice.message.role == "assistant"
    assert choice.message.content is not None
    # MockEngine emits deterministic word sequence starting with 'the quick brown fox'
    assert "the quick brown fox" in choice.message.content
    assert completion.usage is not None
    assert completion.usage.prompt_tokens > 0
    assert completion.usage.completion_tokens > 0
    assert completion.usage.total_tokens == completion.usage.prompt_tokens + completion.usage.completion_tokens


def test_chat_completion_stream(client, model_id):
    response = client.chat.completions.create(
        model=model_id,
        messages=[{"role": "user", "content": "Stream to me"}],
        stream=True,
        stream_options={"include_usage": True},
    )
    collected_deltas = []
    finish_reason_seen = None
    usage_seen = None

    for chunk in response:
        assert chunk.object == "chat.completion.chunk"
        if chunk.choices and len(chunk.choices) > 0:
            c = chunk.choices[0]
            if c.delta and c.delta.content:
                collected_deltas.append(c.delta.content)
            if c.finish_reason:
                finish_reason_seen = c.finish_reason
        if chunk.usage:
            usage_seen = chunk.usage

    full_text = "".join(collected_deltas)
    assert len(full_text) > 0
    assert "the quick brown fox" in full_text
    assert finish_reason_seen == "stop"
    assert usage_seen is not None
    assert usage_seen.total_tokens > 0


def test_tools_round_trip(client, model_id):
    tools = [
        {
            "type": "function",
            "function": {
                "name": "calculator",
                "description": "Evaluate basic arithmetic",
                "parameters": {
                    "type": "object",
                    "properties": {"expression": {"type": "string"}},
                    "required": ["expression"],
                },
            },
        }
    ]
    completion = client.chat.completions.create(
        model=model_id,
        messages=[{"role": "user", "content": "What is 23 * 7?"}],
        tools=tools,
    )
    assert completion.object == "chat.completion"
    assert len(completion.choices) == 1
    choice = completion.choices[0]
    # Note: MockEngine generates text rather than parsing function call semantics;
    # this assertion verifies request/response schema plumbing round-trip through SDK.
    assert choice.message.role == "assistant"
    assert choice.finish_reason in ("stop", "tool_calls")
    if choice.finish_reason == "stop":
        assert choice.message.content is not None
        assert len(choice.message.content) > 0


def test_embeddings(client, model_id):
    res = client.embeddings.create(
        model=model_id,
        input="The quick brown fox jumps over the lazy dog.",
    )
    assert res.object == "list"
    assert len(res.data) == 1
    emb = res.data[0]
    assert emb.object == "embedding"
    assert emb.index == 0
    assert isinstance(emb.embedding, list)
    assert len(emb.embedding) > 0
    assert all(isinstance(v, float) for v in emb.embedding)
    assert res.usage.total_tokens > 0
