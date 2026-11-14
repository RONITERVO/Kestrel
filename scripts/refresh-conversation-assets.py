"""Maintainer-only manifest refresh. Never shipped as a runtime download path.

Read immutable official revisions, use LFS SHA-256 or hash small Git files, and write the
allowlist consumed by native Setup. Run deliberately when updating the pinned speech release.
"""
import hashlib
import json
from pathlib import Path
import urllib.request

ROOT = Path(__file__).resolve().parents[1] / "src-tauri/resources/conversation_voice"
REPO = "kyutai/pocket-tts-without-voice-cloning"
LANGUAGES = {"en": ("english", "alba"), "fr": ("french", "estelle"),
             "de": ("german", "juergen"), "pt": ("portuguese", "rafael"),
             "it": ("italian", "giovanni"), "es": ("spanish", "lola"), "nl": ("dutch", "daan")}


def read(url):
    with urllib.request.urlopen(url, timeout=60) as response:
        return response.read()


def asset(repo, revision, path, target):
    parent = str(Path(path).parent).replace("\\", "/")
    suffix = "" if parent == "." else "/" + parent
    files = json.loads(read(f"https://huggingface.co/api/models/{repo}/tree/{revision}{suffix}"))
    item = next(item for item in files if item["path"] == path)
    url = f"https://huggingface.co/{repo}/resolve/{revision}/{path}"
    digest = item.get("lfs", {}).get("oid")
    if not digest:
        if item["size"] > 4_000_000:
            raise ValueError("Large asset has no SHA-256")
        digest = hashlib.sha256(read(url)).hexdigest()
    if len(digest) != 64 or any(c not in "0123456789abcdef" for c in digest):
        raise ValueError("Upstream did not expose an authorized SHA-256")
    return {"relative": target, "url": url, "bytes": item["size"], "sha256": digest}


def main():
    assets = []
    for code, (language, voice) in LANGUAGES.items():
        weights = "a7056f505a4a8ca72d8b5fb1440071d6f1a35739" if code == "en" else "1e08e6a23401048648a9fdcfde2f89348215c2a7"
        tokenizer = ("00eac05ed3d16bdc3f6b5d598874019c34a89214" if code == "en" else
                     "8843db76457a91db32077edf8dfcd1c0e3e755fd" if code == "fr" else
                     "4e1e0a3e611c51c0b4ed8174fc10f32a54644303")
        assets.append(asset(REPO, weights, f"languages/{language}/model.safetensors", f"models/{code}/model.safetensors"))
        assets.append(asset(REPO, tokenizer, f"languages/{language}/tokenizer.json", f"models/{code}/tokenizer.json"))
        assets.append(asset(REPO, "a7056f505a4a8ca72d8b5fb1440071d6f1a35739",
                            f"languages/{language}/embeddings/{voice}.safetensors", f"models/{code}/voice.safetensors"))
    for file in ["config.json", "tokenizer.json", "model.safetensors"]:
        assets.append(
            asset(
                "openai/whisper-medium.en",
                "2e98eb6279edf5095af0c8dedb36bdec0acd172b",
                file,
                f"models/whisper-medium-en-native/{file}",
            )
        )
    (ROOT / "assets.json").write_text(json.dumps(assets, indent=2) + "\n", encoding="utf-8")
    print(f"Pinned {len(assets)} speech assets, {sum(item['bytes'] for item in assets):,} bytes")


if __name__ == "__main__":
    main()
