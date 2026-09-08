#!/usr/bin/env python3
"""One-shot title generator: reads JSON from stdin, writes JSON to stdout."""
import json, sys, argparse
from llama_cpp import Llama

DEFAULT_PROMPT = """Generate a short title for each coding session. Be literal and specific. Mention the project name if present.

Session: fix the login page it keeps redirecting to the wrong URL
Title: fix login page redirect

Session: add dark mode toggle to the settings panel
Title: add dark mode toggle

Session: the database is slow can you add an index to the users table
Title: add users table index

Session: {text}
Title:"""

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", required=True)
    parser.add_argument("--prompt-template", default=None)
    args = parser.parse_args()

    llm = Llama(model_path=args.model, n_ctx=2048, n_threads=4, verbose=False)

    text = json.loads(sys.stdin.read()).get("text", "")[:800]
    template = args.prompt_template or DEFAULT_PROMPT
    prompt = template.replace("{text}", text[:600])

    out = llm(prompt, max_tokens=25, temperature=0.2, stop=["\n", "Session:", "Title:"], echo=False)
    title = out["choices"][0]["text"].strip().strip('"').strip()
    print(json.dumps({"title": title}))

if __name__ == "__main__":
    main()
