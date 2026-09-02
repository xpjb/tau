import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { open, realpath, stat } from "node:fs/promises";
import { basename, isAbsolute, relative, resolve } from "node:path";

const IMAGE_LIMIT = 10_000_000;
const FILE_LIMIT = 50_000_000;
const CAPTION_LIMIT = 1_024;
const DEFAULT_ROOT = "/root/.local/share/tau/outbox";

type Kind = "image" | "file";

const parameters = Type.Object({
  path: Type.String({ minLength: 1, description: "Path to a file in the Tau outbox" }),
  caption: Type.Optional(Type.String({ description: "Plain-text caption" })),
});

export default function (pi: ExtensionAPI) {
  register(pi, "send_image", "image", "Send a PNG, JPEG, or WebP image to the user through Tau");
  register(pi, "send_file", "file", "Send a downloadable file to the user through Tau");
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

function register(
  pi: ExtensionAPI,
  name: "send_image" | "send_file",
  kind: Kind,
  description: string,
) {
  const root = process.env.TAU_ATTACHMENT_ROOT ?? DEFAULT_ROOT;
  const fullDescription = `${description}. Stage the file under ${root} first.`;
  pi.registerTool({
    name,
    label: kind === "image" ? "Send Image" : "Send File",
    description: fullDescription,
    promptSnippet: fullDescription,
    promptGuidelines: kind === "image"
      ? ["Use send_image when a relevant visual artifact will materially help the user."]
      : ["Use send_file when the user asks to receive a build, archive, report, log, or other file."],
    parameters,
    async execute(_toolCallId, params, signal, _onUpdate, context) {
      signal?.throwIfAborted();
      const attachment = await validate(kind, params.path, params.caption, context);
      signal?.throwIfAborted();
      return {
        content: [{
          type: "text" as const,
          text: `${kind === "image" ? "Image" : "File"} queued for Tau: ${basename(attachment.path)}`,
        }],
        details: {
          tauAttachment: {
            version: 1,
            kind,
            path: attachment.path,
            caption: attachment.caption,
          },
        },
      };
    },
  });
}

async function validate(
  kind: Kind,
  inputPath: string,
  caption: string | undefined,
  context: ExtensionContext,
) {
  const cleaned = inputPath.startsWith("@") ? inputPath.slice(1) : inputPath;
  const candidate = isAbsolute(cleaned) ? cleaned : resolve(context.cwd, cleaned);
  const path = await realpath(candidate).catch(() => {
    throw new Error(`Attachment does not exist: ${inputPath}`);
  });
  const metadata = await stat(path);
  if (!metadata.isFile()) throw new Error("Attachment must be a regular file");

  const configuredRoot = process.env.TAU_ATTACHMENT_ROOT ?? DEFAULT_ROOT;
  if (!isAbsolute(configuredRoot)) throw new Error("Tau attachment root must be absolute");
  const root = await realpath(configuredRoot).catch(() => {
    throw new Error(`Tau attachment root does not exist: ${configuredRoot}`);
  });
  const child = relative(root, path);
  if (child !== "" && (child.startsWith("..") || isAbsolute(child))) {
    throw new Error(`Attachment is outside the Tau outbox: ${root}`);
  }

  const limit = kind === "image" ? IMAGE_LIMIT : FILE_LIMIT;
  if (metadata.size > limit) {
    throw new Error(`Attachment is ${metadata.size} bytes; limit is ${limit} bytes`);
  }
  if (kind === "image" && !(await imageMime(path))) {
    throw new Error("send_image accepts PNG, JPEG, or WebP files");
  }
  const normalizedCaption = caption?.trim() || undefined;
  if (normalizedCaption && Array.from(normalizedCaption).length > CAPTION_LIMIT) {
    throw new Error(`Caption exceeds ${CAPTION_LIMIT} characters`);
  }
  return { path, caption: normalizedCaption };
}

async function imageMime(path: string): Promise<string | undefined> {
  const handle = await open(path, "r");
  try {
    const bytes = Buffer.alloc(12);
    const { bytesRead } = await handle.read(bytes, 0, bytes.length, 0);
    const header = bytes.subarray(0, bytesRead);
    if (header.length >= 8 && header.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) return "image/png";
    if (header.length >= 3 && header[0] === 0xff && header[1] === 0xd8 && header[2] === 0xff) return "image/jpeg";
    if (header.length >= 12 && header.subarray(0, 4).toString("ascii") === "RIFF" && header.subarray(8, 12).toString("ascii") === "WEBP") return "image/webp";
    return undefined;
  } finally {
    await handle.close();
  }
}
