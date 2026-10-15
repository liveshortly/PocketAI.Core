import test from 'node:test';
import assert from 'node:assert/strict';
import OpenAI from 'openai';

const baseURL = process.env.OPENAI_BASE_URL || 'http://127.0.0.1:7480/v1';
const apiKey = process.env.OPENAI_API_KEY || 'test-key';

const client = new OpenAI({ baseURL, apiKey });

test('models list (non-empty)', async () => {
  const models = await client.models.list();
  assert.equal(models.object, 'list');
  assert.ok(models.data.length > 0, 'models list should be non-empty');
  assert.ok(models.data[0].id, 'model should have an id');
  assert.equal(models.data[0].object, 'model');
  assert.equal(models.data[0].owned_by, 'pocketai');
});

test('chat completion non-stream', async () => {
  const models = await client.models.list();
  const modelId = models.data[0].id;

  const res = await client.chat.completions.create({
    model: modelId,
    messages: [{ role: 'user', content: 'Hello PocketAI' }],
    stream: false,
  });

  assert.equal(res.object, 'chat.completion');
  assert.equal(res.model, modelId);
  assert.equal(res.choices.length, 1);
  assert.equal(res.choices[0].finish_reason, 'stop');
  assert.equal(res.choices[0].message.role, 'assistant');
  // MockEngine emits deterministic word sequence
  assert.ok(res.choices[0].message.content.includes('the quick brown fox'));
  assert.ok(res.usage.prompt_tokens > 0);
  assert.ok(res.usage.completion_tokens > 0);
  assert.equal(res.usage.total_tokens, res.usage.prompt_tokens + res.usage.completion_tokens);
});

test('chat completion stream with usage', async () => {
  const models = await client.models.list();
  const modelId = models.data[0].id;

  const stream = await client.chat.completions.create({
    model: modelId,
    messages: [{ role: 'user', content: 'stream to me' }],
    stream: true,
    stream_options: { include_usage: true },
  });

  let fullContent = '';
  let finishReason = null;
  let usage = null;

  for await (const chunk of stream) {
    assert.equal(chunk.object, 'chat.completion.chunk');
    if (chunk.choices && chunk.choices.length > 0) {
      const delta = chunk.choices[0].delta;
      if (delta && delta.content) {
        fullContent += delta.content;
      }
      if (chunk.choices[0].finish_reason) {
        finishReason = chunk.choices[0].finish_reason;
      }
    }
    if (chunk.usage) {
      usage = chunk.usage;
    }
  }

  assert.ok(fullContent.includes('the quick brown fox'));
  assert.equal(finishReason, 'stop');
  assert.ok(usage, 'stream should provide usage');
  assert.ok(usage.total_tokens > 0);
});

test('tools round-trip', async () => {
  const models = await client.models.list();
  const modelId = models.data[0].id;

  const tools = [
    {
      type: 'function',
      function: {
        name: 'calculator',
        description: 'Evaluate basic arithmetic',
        parameters: {
          type: 'object',
          properties: { expression: { type: 'string' } },
          required: ['expression'],
        },
      },
    },
  ];

  const res = await client.chat.completions.create({
    model: modelId,
    messages: [{ role: 'user', content: 'What is 23 * 7?' }],
    tools,
  });

  assert.equal(res.object, 'chat.completion');
  assert.equal(res.choices.length, 1);
  // Shape/plumbing check: mock engine responds with text rather than calling tool
  assert.equal(res.choices[0].message.role, 'assistant');
  assert.ok(['stop', 'tool_calls'].includes(res.choices[0].finish_reason));
  if (res.choices[0].finish_reason === 'stop') {
    assert.ok(res.choices[0].message.content.length > 0);
  }
});

test('embeddings', async () => {
  const models = await client.models.list();
  const modelId = models.data[0].id;

  const res = await client.embeddings.create({
    model: modelId,
    input: 'The quick brown fox jumps over the lazy dog.',
  });

  assert.equal(res.object, 'list');
  assert.equal(res.data.length, 1);
  assert.equal(res.data[0].object, 'embedding');
  assert.equal(res.data[0].index, 0);
  assert.ok(Array.isArray(res.data[0].embedding));
  assert.ok(res.data[0].embedding.length > 0);
  assert.equal(typeof res.data[0].embedding[0], 'number');
  assert.ok(res.usage.total_tokens > 0);
});
