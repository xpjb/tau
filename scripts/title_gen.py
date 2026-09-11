#!/usr/bin/env python3
"""One-shot title generator: reads JSON from stdin, writes JSON to stdout."""
import json, sys, argparse
from pathlib import Path
from llama_cpp import Llama

DEFAULT_PROMPT = Path(__file__).with_name("title_prompt.txt").read_text(encoding="utf-8")

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", required=True)
    parser.add_argument("--prompt-template", default=None)
    args = parser.parse_args()

    llm = Llama(model_path=args.model, n_ctx=2048, n_threads=4, verbose=False)

    request = json.loads(sys.stdin.read())
    template = request.get("promptTemplate")
    if template is None:
        template = args.prompt_template or DEFAULT_PROMPT
    prompt = template.replace("{text}", request.get("text", "")[:600])

    out = llm(prompt, max_tokens=25, temperature=0.2, stop=["\n", "Session:", "Title:"], echo=False)
    title = out["choices"][0]["text"].strip().strip('"').strip()
    print(json.dumps({"title": title}))

if __name__ == "__main__":
    main()
