import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { randomUUID } from "node:crypto";
import { chmod, copyFile, mkdtemp, open, realpath, rm, stat, writeFile } from "node:fs/promises";
import { basename, isAbsolute, join, relative, resolve } from "node:path";

const IMAGE_LIMIT = 10_000_000;
const FILE_LIMIT = 50_000_000;
const CAPTION_LIMIT = 1_024;
const IMAGE_PROMPT_LIMIT = 32_000;
const CODEX_RESPONSE_LIMIT = 32 * 1024 * 1024;
const DEFAULT_ROOT = "/root/.local/share/tau/outbox";

export default function (pi: ExtensionAPI) {
  const description = "Send a local file to the user through Tau. Files are staged automatically; PNG, JPEG, and WebP files up to 10 MB appear inline.";
  pi.registerTool({
    name: "send_file",
    label: "Send File",
    description,
    promptSnippet: description,
    promptGuidelines: ["Use send_file when a relevant visual artifact will materially help the user or when the user asks to receive a build, archive, report, log, or other file."],
    parameters: Type.Object({
      path: Type.String({ minLength: 1, description: "Path to a local regular file" }),
      caption: Type.Optional(Type.String({ description: "Plain-text caption" })),
    }),
    async execute(_toolCallId, params, signal, _onUpdate, context) {
      signal?.throwIfAborted();
      const caption = params.caption?.trim() || undefined;
      if (caption && Array.from(caption).length > CAPTION_LIMIT) {
        throw new Error(`Caption exceeds ${CAPTION_LIMIT} characters`);
      }
      const cleaned = params.path.startsWith("@") ? params.path.slice(1) : params.path;
      const candidate = isAbsolute(cleaned) ? cleaned : resolve(context.cwd, cleaned);
      const source = await realpath(candidate).catch(() => {
        throw new Error(`Attachment does not exist: ${params.path}`);
      });
      const sourceMetadata = await stat(source);
      if (!sourceMetadata.isFile()) throw new Error("Attachment must be a regular file");
      if (sourceMetadata.size > FILE_LIMIT) {
        throw new Error(`Attachment is ${sourceMetadata.size} bytes; limit is ${FILE_LIMIT} bytes`);
      }

      const configuredRoot = process.env.TAU_ATTACHMENT_ROOT ?? DEFAULT_ROOT;
      if (!isAbsolute(configuredRoot)) throw new Error("Tau attachment root must be absolute");
      const root = await realpath(configuredRoot).catch(() => {
        throw new Error(`Tau attachment root does not exist: ${configuredRoot}`);
      });
      const child = relative(root, source);
      let path = source;
      let stagedDirectory: string | undefined;
      try {
        if (child !== "" && (child.startsWith("..") || isAbsolute(child))) {
          stagedDirectory = await mkdtemp(join(root, ".tau-"));
          path = join(stagedDirectory, basename(source));
          await copyFile(source, path);
          await chmod(path, 0o600);
        }

        signal?.throwIfAborted();
        const metadata = await stat(path);
        if (!metadata.isFile()) throw new Error("Staged attachment must be a regular file");
        if (metadata.size > FILE_LIMIT) {
          throw new Error(`Attachment is ${metadata.size} bytes after staging; limit is ${FILE_LIMIT} bytes`);
        }
        const handle = await open(path, "r");
        let image = false;
        try {
          const bytes = Buffer.alloc(12);
          const { bytesRead } = await handle.read(bytes, 0, bytes.length, 0);
          const header = bytes.subarray(0, bytesRead);
          image = (header.length >= 8 && header.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])))
            || (header.length >= 3 && header[0] === 0xff && header[1] === 0xd8 && header[2] === 0xff)
            || (header.length >= 12 && header.subarray(0, 4).toString("ascii") === "RIFF" && header.subarray(8, 12).toString("ascii") === "WEBP");
        } finally {
          await handle.close();
        }
        const kind = image && metadata.size <= IMAGE_LIMIT ? "image" : "file";
        signal?.throwIfAborted();
        return {
          content: [{
            type: "text" as const,
            text: `${kind === "image" ? "Image" : "File"} queued for Tau: ${basename(path)}`,
          }],
          details: {
            tauAttachment: { version: 1, kind, path, caption, size: metadata.size },
          },
        };
      } catch (error) {
        if (stagedDirectory) await rm(stagedDirectory, { recursive: true, force: true }).catch(() => {});
        throw error;
      }
    },
  });

  const generateDescription = "Generate one new PNG with OpenAI's gpt-image-2 and send it to the user through Tau.";
  pi.registerTool({
    name: "generate_image",
    label: "Generate Image",
    description: generateDescription,
    promptSnippet: generateDescription,
    promptGuidelines: [
      "Use generate_image when the user asks for a new illustration, picture, or other generative image. Put all requested visual details in one complete prompt.",
      "One call creates one image. Do not retry an unconfirmed generation unless the user explicitly asks, because OpenAI may have consumed the image allowance.",
    ],
    executionMode: "sequential",
    parameters: Type.Object({
      prompt: Type.String({ minLength: 1, maxLength: IMAGE_PROMPT_LIMIT, description: "Complete prompt for the image" }),
    }),
    async execute(_toolCallId, params, signal, _onUpdate, context) {
      signal?.throwIfAborted();
      const prompt = params.prompt.trim();
      if (!prompt || Array.from(prompt).length > IMAGE_PROMPT_LIMIT) {
        throw new Error(`Image prompt must contain 1–${IMAGE_PROMPT_LIMIT} characters`);
      }

      const configuredRoot = process.env.TAU_ATTACHMENT_ROOT ?? DEFAULT_ROOT;
      if (!isAbsolute(configuredRoot)) throw new Error("Tau attachment root must be absolute");
      const root = await realpath(configuredRoot).catch(() => {
        throw new Error(`Tau attachment root does not exist: ${configuredRoot}`);
      });

      const auth = await context.modelRegistry.getProviderAuth("openai-codex").catch(() => undefined);
      const token = auth?.auth.apiKey;
      if (!token) throw new Error("OpenAI image generation requires Pi's Codex sign-in");
      let account: unknown;
      try {
        const payload = JSON.parse(Buffer.from(token.split(".")[1] ?? "", "base64url").toString("utf8"));
        account = payload?.["https://api.openai.com/auth"]?.chatgpt_account_id;
      } catch {}
      if (typeof account !== "string" || !/^[a-z0-9_-]{1,128}$/i.test(account)) {
        throw new Error("Pi's Codex login has no valid ChatGPT account ID");
      }
      const model = context.model?.provider === "openai-codex"
        ? context.model.id
        : context.modelRegistry.find("openai-codex", "gpt-6-sol")?.id
          ?? context.modelRegistry.getAvailable().find((candidate) => candidate.provider === "openai-codex")?.id;
      if (!model) throw new Error("No OpenAI Codex model is available to start image generation");

      const headers = new Headers();
      for (const [name, value] of Object.entries(auth.auth.headers ?? {})) {
        if (typeof value === "string") headers.set(name, value);
      }
      const requestId = randomUUID();
      headers.set("Authorization", `Bearer ${token}`);
      headers.set("ChatGPT-Account-Id", account);
      headers.set("Originator", "tau");
      headers.set("User-Agent", "tau");
      headers.set("OpenAI-Beta", "responses=experimental");
      headers.set("Accept", "text/event-stream");
      headers.set("Content-Type", "application/json");
      headers.set("Session-Id", requestId);
      headers.set("X-Client-Request-Id", requestId);

      const deadline = AbortSignal.timeout(10 * 60_000);
      const requestSignal = signal ? AbortSignal.any([signal, deadline]) : deadline;
      let image: Buffer;
      try {
        const response = await fetch("https://chatgpt.com/backend-api/codex/responses", {
          method: "POST",
          redirect: "error",
          headers,
          body: JSON.stringify({
            model,
            store: false,
            stream: true,
            instructions: "Generate exactly one image from the user's prompt. Use image_generation once and return no prose.",
            input: [{ role: "user", content: [{ type: "input_text", text: prompt }] }],
            tools: [{ type: "image_generation", model: "gpt-image-2", output_format: "png" }],
            tool_choice: "required",
            parallel_tool_calls: false,
            reasoning: { effort: "low", summary: "auto" },
            text: { verbosity: "low" },
          }),
          signal: requestSignal,
        });
        if (!response.ok) {
          await response.body?.cancel();
          throw new Error(`OpenAI returned HTTP ${response.status}`);
        }
        if (!response.body) throw new Error("OpenAI returned no response body");
        const declaredBytes = Number(response.headers.get("content-length"));
        if (Number.isFinite(declaredBytes) && declaredBytes > CODEX_RESPONSE_LIMIT) {
          await response.body.cancel();
          throw new Error("OpenAI response exceeded its size limit");
        }
        const reader = response.body.getReader();
        const decoder = new TextDecoder();
        let wire = "";
        let wireBytes = 0;
        while (true) {
          const chunk = await reader.read();
          if (chunk.done) break;
          wireBytes += chunk.value.byteLength;
          if (wireBytes > CODEX_RESPONSE_LIMIT) {
            await reader.cancel();
            throw new Error("OpenAI response exceeded its size limit");
          }
          wire += decoder.decode(chunk.value, { stream: true });
        }
        wire += decoder.decode();

        let terminal = false;
        let imageId: string | undefined;
        let encoded: string | undefined;
        for (const block of wire.split(/\r?\n\r?\n/)) {
          const data = block.split(/\r?\n/)
            .filter((line) => line.startsWith("data:"))
            .map((line) => line.slice(5).replace(/^ /, ""))
            .join("\n");
          if (!data || data === "[DONE]") continue;
          let event: any;
          try {
            event = JSON.parse(data);
          } catch {
            throw new Error("OpenAI returned malformed image events");
          }
          if (event?.type === "error" || event?.type === "response.failed") {
            const code = event?.error?.code ?? event?.response?.error?.code ?? event?.code;
            throw new Error(`OpenAI image generation failed${typeof code === "string" ? ` (${code})` : ""}`);
          }

          let items: any[] = [];
          if (event?.type === "response.output_item.done") items = [event.item];
          if (["response.completed", "response.done", "response.incomplete"].includes(event?.type)) {
            terminal = true;
            if (event.type === "response.incomplete" || event?.response?.status !== "completed") {
              throw new Error("OpenAI did not complete image generation");
            }
            if (Array.isArray(event.response.output)) items = event.response.output;
          }
          for (const item of items) {
            if (item?.type !== "image_generation_call") continue;
            if (item.status !== "completed") throw new Error("OpenAI did not complete the generated image");
            if (typeof item.id !== "string" || !item.id) throw new Error("OpenAI returned an image without an ID");
            if (typeof item.result !== "string" || !item.result) throw new Error("OpenAI returned no generated image data");
            if (imageId === item.id) {
              if (encoded !== item.result) throw new Error("OpenAI returned inconsistent generated image data");
              continue;
            }
            if (imageId) throw new Error("OpenAI returned more than one generated image");
            imageId = item.id;
            encoded = item.result;
          }
        }
        if (!terminal) throw new Error("OpenAI image stream ended before completion");
        if (!encoded || !imageId) throw new Error("OpenAI completed without a generated image");
        if (encoded.length > Math.ceil(IMAGE_LIMIT / 3) * 4 || encoded.length % 4 !== 0
          || !/^[A-Za-z0-9+/]*={0,2}$/.test(encoded)) {
          throw new Error("OpenAI returned invalid or oversized image data");
        }
        image = Buffer.from(encoded, "base64");
        if (!image.length || image.length > IMAGE_LIMIT || image.toString("base64") !== encoded
          || !image.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) {
          throw new Error("OpenAI returned invalid or oversized PNG data");
        }
      } catch (error) {
        if (signal?.aborted) signal.throwIfAborted();
        const reason = deadline.aborted ? "The request timed out." : error instanceof Error ? error.message : "The request failed.";
        throw new Error(`OpenAI image generation was not confirmed. Do not retry automatically; ask the user before another generation request. ${reason}`);
      }

      signal?.throwIfAborted();
      let stagedDirectory: string | undefined;
      try {
        stagedDirectory = await mkdtemp(join(root, ".tau-image-"));
        const path = join(stagedDirectory, "generated-image.png");
        await writeFile(path, image, { flag: "wx", mode: 0o600 });
        await chmod(path, 0o600);
        signal?.throwIfAborted();
        return {
          content: [{ type: "text" as const, text: "Generated image queued for Tau: generated-image.png" }],
          details: {
            tauAttachment: { version: 1, kind: "image", path, size: image.length },
          },
        };
      } catch (error) {
        if (stagedDirectory) await rm(stagedDirectory, { recursive: true, force: true }).catch(() => {});
        if (signal?.aborted) signal.throwIfAborted();
        const reason = error instanceof Error ? error.message : "The file write failed.";
        throw new Error(`OpenAI generated the image, but Tau could not stage it. Do not retry automatically; ask the user before another generation request. ${reason}`);
      }
    },
  });

  const flagUrl = process.env.TAU_FLAG_URL;
  const flagToken = process.env.TAU_FLAG_TOKEN;
  if (flagUrl && flagToken) pi.registerTool({
    name: "flag_it",
    label: "Flag It",
    description: "Log an incidental issue outside the current task's scope for the user to investigate later.",
    promptSnippet: "Flag incidental technical debt, environment problems or operational inefficiency without changing task scope.",
    promptGuidelines: [
      "Use flag_it for new, actionable findings outside the current task. State what you observed, where, and why it matters; distinguish facts from suspicions and omit secrets.",
      "Continue the current task after flagging. A flag does not authorize investigation or extra work. Do not repeatedly flag the same known issue or bypass this tool by editing the daemon's flag log.",
    ],
    parameters: Type.Object({ str: Type.String({ minLength: 1, maxLength: 4096, description: "The finding, location and impact; no secrets" }) }),
    async execute(_toolCallId, params, signal) {
      signal?.throwIfAborted();
      if (!params.str.trim() || Array.from(params.str).length > 4096) throw new Error("Flag text must contain 1–4096 characters");
      const deadline = AbortSignal.timeout(30_000);
      try {
        const response = await fetch(flagUrl, {
          method: "POST", redirect: "error",
          headers: { Authorization: `Bearer ${flagToken}`, "Content-Type": "application/json" },
          body: JSON.stringify({ text: params.str }),
          signal: signal ? AbortSignal.any([signal, deadline]) : deadline,
        });
        if (!response.ok) throw new Error(`Tau returned HTTP ${response.status}`);
        const saved: unknown = await response.json();
        if (!saved || typeof saved !== "object" || !("id" in saved) || typeof saved.id !== "string" || !saved.id) {
          throw new Error("Tau returned no flag ID");
        }
        return { content: [{ type: "text" as const, text: `Flag ${saved.id} saved in Tau's flags.jsonl. Continue the current task.` }],
          details: { flagId: saved.id } };
      } catch (error) {
        throw new Error(`Flag save unconfirmed; check Tau's flags.jsonl before retrying. ${error instanceof Error ? error.message : String(error)}`);
      }
    },
  });
  pi.registerCommand("tau-fork-at", {
    description: "Create a Tau fork through the selected session entry",
    handler: async (args, context) => {
      const entryId = args.trim();
      if (!entryId || entryId.length > 256 || /\s/.test(entryId)) {
        throw new Error("Tau fork entry ID is invalid");
      }
      const result = await context.fork(entryId, { position: "at" });
      if (result.cancelled) throw new Error("Pi cancelled the Tau fork");
    },
  });
}
