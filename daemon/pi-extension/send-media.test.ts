import assert from "node:assert/strict";
import { once } from "node:events";
import { mkdtemp, mkdir, open, readFile, realpath, rm, stat, writeFile } from "node:fs/promises";
import { createServer, type ServerResponse } from "node:http";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { test } from "node:test";
import type { ExtensionAPI, ExtensionContext, ToolDefinition } from "@earendil-works/pi-coding-agent";
import register from "./send-media.ts";

test("Tau media tools stage local files and generated OpenAI images", async () => {
  const previousRoot = process.env.TAU_ATTACHMENT_ROOT;
  const previousUrl = process.env.TAU_FLAG_URL;
  const previousToken = process.env.TAU_FLAG_TOKEN;
  const previousFetch = globalThis.fetch;
  const directory = await mkdtemp(join(tmpdir(), "tau-send-file-"));
  const outbox = join(directory, "outbox");
  await mkdir(outbox);
  try {
    process.env.TAU_ATTACHMENT_ROOT = outbox;
    delete process.env.TAU_FLAG_URL;
    delete process.env.TAU_FLAG_TOKEN;
    const tools: ToolDefinition[] = [];
    const api = { registerTool: (tool: ToolDefinition) => { tools.push(tool); }, registerCommand: () => {} } as unknown as ExtensionAPI;
    register(api);
    assert.deepEqual(tools.map((tool) => tool.name), ["send_file", "generate_image"]);
    const send = tools[0];
    const context = { cwd: directory } as ExtensionContext;

    const source = join(directory, "report.txt");
    await writeFile(source, "original report");
    const result = await send.execute("file", { path: "report.txt", caption: " Report " }, undefined, undefined, context);
    const attachment = (result.details as { tauAttachment: { kind: string; path: string; caption?: string; size: number } }).tauAttachment;
    assert.equal(attachment.kind, "file");
    assert.equal(attachment.caption, "Report");
    assert.equal(attachment.size, 15);
    assert.equal(basename(attachment.path), "report.txt");
    assert.notEqual(attachment.path, source);
    assert(attachment.path.startsWith(`${await realpath(outbox)}/`));
    assert.equal((await stat(attachment.path)).mode & 0o777, 0o600);
    await writeFile(source, "changed");
    assert.equal(await readFile(attachment.path, "utf8"), "original report");

    const png = join(directory, "pixel.png");
    await writeFile(png, Buffer.from([137, 80, 78, 71, 13, 10, 26, 10, 0]));
    const imageResult = await send.execute("image", { path: png }, undefined, undefined, context);
    const image = (imageResult.details as { tauAttachment: { kind: string; path: string; size: number } }).tauAttachment;
    assert.equal(image.kind, "image");
    assert.equal(image.size, 9);
    assert.equal(basename(image.path), "pixel.png");

    const largePng = join(directory, "large.png");
    const largePngHandle = await open(largePng, "w");
    try {
      await largePngHandle.write(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), 0, 8, 0);
      await largePngHandle.truncate(10_000_001);
    } finally {
      await largePngHandle.close();
    }
    const largeImageResult = await send.execute("large-image", { path: largePng }, undefined, undefined, context);
    const largeImage = (largeImageResult.details as { tauAttachment: { kind: string; size: number } }).tauAttachment;
    assert.equal(largeImage.kind, "file");
    assert.equal(largeImage.size, 10_000_001);

    const staged = join(outbox, "ready.bin");
    await writeFile(staged, "ready");
    const stagedResult = await send.execute("staged", { path: staged }, undefined, undefined, context);
    const reused = (stagedResult.details as { tauAttachment: { path: string } }).tauAttachment;
    assert.equal(reused.path, await realpath(staged));
    await assert.rejects(send.execute("caption", { path: staged, caption: "x".repeat(1_025) }, undefined, undefined, context), /Caption exceeds/);

    const generatedPng = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6kAAAAABJRU5ErkJggg==", "base64");
    const token = `header.${Buffer.from(JSON.stringify({
      "https://api.openai.com/auth": { chatgpt_account_id: "account-test" },
    })).toString("base64url")}.signature`;
    const requests: { url: string; headers: Headers; body: any }[] = [];
    let failGeneration = false;
    globalThis.fetch = (async (input, init) => {
      requests.push({
        url: String(input),
        headers: new Headers(init?.headers),
        body: JSON.parse(String(init?.body)),
      });
      if (failGeneration) return new Response("failure", { status: 503 });
      const item = { type: "image_generation_call", id: "ig-test", status: "completed", result: generatedPng.toString("base64") };
      const events = [
        { type: "response.output_item.done", output_index: 0, item },
        { type: "response.completed", response: { status: "completed", output: [] } },
      ];
      return new Response(events.map((event) => `event: ${event.type}\r\ndata: ${JSON.stringify(event)}\r\n\r\n`).join(""), {
        status: 200,
        headers: { "Content-Type": "text/event-stream" },
      });
    }) as typeof fetch;
    const generate = tools[1];
    const generateContext = {
      cwd: directory,
      model: { provider: "openai-codex", id: "gpt-test" },
      modelRegistry: { getProviderAuth: async () => ({ auth: { apiKey: token } }) },
    } as unknown as ExtensionContext;
    const generatedResult = await generate.execute("generate", { prompt: " A small orange triangle. " }, undefined, undefined, generateContext);
    const generated = (generatedResult.details as { tauAttachment: { kind: string; path: string; size: number } }).tauAttachment;
    assert.equal(generated.kind, "image");
    assert.equal(generated.size, generatedPng.length);
    assert.equal(basename(generated.path), "generated-image.png");
    assert(generated.path.startsWith(`${await realpath(outbox)}/`));
    assert.deepEqual(await readFile(generated.path), generatedPng);
    assert.equal((await stat(generated.path)).mode & 0o777, 0o600);
    assert.equal(requests.length, 1);
    assert.equal(requests[0].url, "https://chatgpt.com/backend-api/codex/responses");
    assert.equal(requests[0].headers.get("authorization"), `Bearer ${token}`);
    assert.equal(requests[0].headers.get("chatgpt-account-id"), "account-test");
    assert.equal(requests[0].body.model, "gpt-test");
    assert.equal(requests[0].body.input[0].content[0].text, "A small orange triangle.");
    assert.deepEqual(requests[0].body.tools, [{ type: "image_generation", model: "gpt-image-2", output_format: "png" }]);
    assert.equal(requests[0].body.tool_choice, "required");
    failGeneration = true;
    await assert.rejects(
      generate.execute("failed-generate", { prompt: "Try once" }, undefined, undefined, generateContext),
      /Do not retry automatically.*HTTP 503/,
    );
    assert.equal(requests.length, 2, "generate_image retried a possibly charged request");
  } finally {
    globalThis.fetch = previousFetch;
    if (previousRoot === undefined) delete process.env.TAU_ATTACHMENT_ROOT; else process.env.TAU_ATTACHMENT_ROOT = previousRoot;
    if (previousUrl === undefined) delete process.env.TAU_FLAG_URL; else process.env.TAU_FLAG_URL = previousUrl;
    if (previousToken === undefined) delete process.env.TAU_FLAG_TOKEN; else process.env.TAU_FLAG_TOKEN = previousToken;
    await rm(directory, { recursive: true, force: true });
  }
});

test("flag_it waits for daemon confirmation and stays scoped to Tau", { timeout: 10_000 }, async () => {
  const previousUrl = process.env.TAU_FLAG_URL;
  const previousToken = process.env.TAU_FLAG_TOKEN;
  const tools: ToolDefinition[] = [];
  const api = { registerTool: (tool: ToolDefinition) => { tools.push(tool); }, registerCommand: () => {} } as unknown as ExtensionAPI;
  const requests: { text: string }[] = [];
  let mode = "hold";
  let held: ServerResponse | undefined;
  const server = createServer(async (request, response) => {
    assert.equal(request.method, "POST");
    assert.equal(request.url, "/v1/sessions/source/flags");
    assert.equal(request.headers.authorization, "Bearer flag-only-test-token");
    assert.equal(request.headers["content-type"], "application/json");
    let body = "";
    for await (const chunk of request) body += chunk;
    requests.push(JSON.parse(body));
    response.setHeader("Content-Type", "application/json");
    if (mode === "hold") { held = response; server.emit("flag-request"); }
    else if (mode === "fail") { response.statusCode = 500; response.end(JSON.stringify({ error: "Disk failure" })); }
    else response.end(JSON.stringify({}));
  });
  try {
    delete process.env.TAU_FLAG_URL;
    delete process.env.TAU_FLAG_TOKEN;
    register(api);
    assert.deepEqual(tools.map((tool) => tool.name), ["send_file", "generate_image"]);
    server.listen(0, "127.0.0.1");
    await once(server, "listening");
    const address = server.address();
    assert(address && typeof address !== "string");
    process.env.TAU_FLAG_URL = `http://127.0.0.1:${address.port}/v1/sessions/source/flags`;
    process.env.TAU_FLAG_TOKEN = "flag-only-test-token";
    tools.length = 0;
    register(api);
    const flag = tools.find((tool) => tool.name === "flag_it");
    assert(flag);
    const context = {} as ExtensionContext;
    const text = "Private cache 🔧\nInspect build-dir overrides, without leaving the current task.";
    const received = once(server, "flag-request");
    let completed = false;
    const pending = flag.execute("call-1", { str: text }, undefined, undefined, context).then((result) => { completed = true; return result; });
    await received;
    assert(!completed, "Tool claimed success before daemon confirmation");
    assert.deepEqual(requests, [{ text }]);
    assert(held);
    held.end(JSON.stringify({ id: "flag-fixture" }));
    const result = await pending;
    assert.deepEqual(result.details, { flagId: "flag-fixture" });
    assert(JSON.stringify(result).includes("Continue the current task"));
    assert(!JSON.stringify(result).includes("flag-only-test-token"));
    for (const str of [" \n ", "x".repeat(4097)]) {
      await assert.rejects(flag.execute("invalid", { str }, undefined, undefined, context), /1–4096 characters/);
    }
    await assert.rejects(flag.execute("cancelled", { str: text }, AbortSignal.abort(), undefined, context), /aborted/i);
    assert.equal(requests.length, 1);
    mode = "fail";
    await assert.rejects(flag.execute("failed", { str: text }, undefined, undefined, context), /Flag save unconfirmed.*HTTP 500/);
    mode = "bad-receipt";
    await assert.rejects(flag.execute("bad", { str: text }, undefined, undefined, context), /Flag save unconfirmed.*no flag ID/);
    mode = "hold";
    const incoming = once(server, "flag-request");
    const stop = new AbortController();
    const cancelled = flag.execute("stopped", { str: text }, stop.signal, undefined, context);
    const rejected = assert.rejects(cancelled, /Flag save unconfirmed/);
    await incoming;
    stop.abort();
    await rejected;
    assert.equal(requests.length, 4, "Tool silently retried a possibly saved flag");
  } finally {
    if (previousUrl === undefined) delete process.env.TAU_FLAG_URL; else process.env.TAU_FLAG_URL = previousUrl;
    if (previousToken === undefined) delete process.env.TAU_FLAG_TOKEN; else process.env.TAU_FLAG_TOKEN = previousToken;
    server.closeAllConnections();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
});
