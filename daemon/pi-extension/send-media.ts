import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { chmod, copyFile, mkdtemp, open, realpath, rm, stat } from "node:fs/promises";
import { basename, isAbsolute, join, relative, resolve } from "node:path";

const IMAGE_LIMIT = 10_000_000;
const FILE_LIMIT = 50_000_000;
const CAPTION_LIMIT = 1_024;
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
