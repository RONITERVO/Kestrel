"""Offline pixel evidence and a local side-by-side review page; never declares quality a pass.

Run with the isolated ComfyUI Python (Pillow, NumPy and SciPy are already installed):
python scripts/image-acceptance-review.py ABSOLUTE_RESULTS_DIRECTORY
"""
from pathlib import Path
import csv
from datetime import datetime, timedelta
import hashlib
import html
import json
import sys

import numpy as np
from PIL import Image
from scipy import ndimage


def shape_measurement(pixels, color, normalized_box):
    rgb = pixels.astype(np.float32)
    r, g, b = rgb[:, :, 0], rgb[:, :, 1], rgb[:, :, 2]
    mask = (r > 100) & (r > 1.5 * g) & (r > 1.5 * b) if color == "red" else (b > 90) & (b > 1.35 * r) & (b > 1.15 * g)
    labels, count = ndimage.label(mask)
    sizes = np.bincount(labels.ravel())
    sizes[0] = 0
    if not count or sizes.max() == 0:
        return {"detected": False, "color": color}
    largest = labels == int(sizes.argmax())
    ys, xs = np.nonzero(largest)
    height, width = mask.shape
    top, left, bottom, right = normalized_box
    target = [int(np.floor(top * height / 1000)), int(np.floor(left * width / 1000)), int(np.ceil(bottom * height / 1000)), int(np.ceil(right * width / 1000))]
    actual = [int(ys.min()), int(xs.min()), int(ys.max() + 1), int(xs.max() + 1)]
    intersection = max(0, min(actual[2], target[2]) - max(actual[0], target[0])) * max(0, min(actual[3], target[3]) - max(actual[1], target[1]))
    area = (actual[2] - actual[0]) * (actual[3] - actual[1])
    target_area = (target[2] - target[0]) * (target[3] - target[1])
    actual_center = np.array([(actual[0] + actual[2]) / 2, (actual[1] + actual[3]) / 2])
    target_center = np.array([(target[0] + target[2]) / 2, (target[1] + target[3]) / 2])
    inside = largest[target[0]:target[2], target[1]:target[3]].sum()
    return {
        "detected": True, "color": color, "requestedPixelBoxYxyx": target, "measuredPixelBoxYxyx": actual,
        "boxIoU": intersection / (area + target_area - intersection),
        "coloredPixelContainment": float(inside / largest.sum()),
        "centerErrorDiagonalFraction": float(np.linalg.norm(actual_center - target_center) / np.hypot(height, width)),
        "substantialColorComponents": int((sizes > width * height * .005).sum()),
    }


def review_picture(root, backend, name, project, take):
    source = html.escape(Path(take["path"]).resolve().relative_to(root).as_posix())
    boxes = []
    for number, element in enumerate(project.get("elements", []), 1):
        top, left, bottom, right = element["bbox"]
        boxes.append(f'<rect x="{left}" y="{top}" width="{right-left}" height="{bottom-top}"/><text x="{left+5}" y="{top+24}">{number}</text>')
    overlay = f'<svg class="overlay" viewBox="0 0 1000 1000" preserveAspectRatio="none">{"".join(boxes)}</svg>'
    return f'<a class="figure" href="{source}"><img loading="lazy" src="{source}" alt="{backend} {html.escape(name)} variation {take["batchIndex"]}">{overlay}</a>'


def main():
    root = Path(sys.argv[1]).resolve(strict=True)
    review_file = root / "visual-review.json"
    reviews = json.loads(review_file.read_text(encoding="utf-8")) if review_file.exists() else {}
    telemetry_file = root / "gpu-telemetry.csv"
    telemetry = []
    if telemetry_file.exists():
        for row in csv.reader(telemetry_file.read_text(encoding="utf-8-sig").splitlines()):
            if len(row) == 6:
                telemetry.append((datetime.fromisoformat(row[0].strip()), int(row[4].strip())))
    records = {}
    evidence = []
    for backend in ("ideogram", "moss"):
        for path in sorted((root / backend).glob("*.json")):
            data = json.loads(path.read_text(encoding="utf-8"))
            if "case" not in data:
                continue
            records.setdefault(data["case"], {})[backend] = data
            review = reviews.get(backend, {}).get(data["case"], {})
            item = {"case": data["case"], "backend": backend, "status": data["status"], "qualityVerdict": review.get("verdict", "pending"), "visualNotes": review.get("notes", ""), "elapsedSeconds": data.get("elapsedSeconds"), "takes": []}
            if telemetry and data.get("project", {}).get("createdAt") and data.get("elapsedSeconds"):
                started = datetime.fromisoformat(data["project"]["createdAt"])
                ended = started + timedelta(seconds=data["elapsedSeconds"])
                samples = [used for at, used in telemetry if started <= at <= ended]
                item["gpuTelemetry"] = {"sampleIntervalSeconds": 2, "coversWholeRender": telemetry[0][0] <= started and telemetry[-1][0] >= ended, "peakUsedMiBObserved": max(samples) if samples else None}
            for take in data.get("project", {}).get("takes", []):
                if take["status"] != "complete":
                    continue
                image = Image.open(take["path"]).convert("RGB")
                pixels = np.asarray(image)
                measured = {"path": take["path"], "size": list(image.size), "fileSha256": hashlib.sha256(Path(take["path"]).read_bytes()).hexdigest(), "decodedPixelSha256": hashlib.sha256(pixels.tobytes()).hexdigest()}
                measured["matchesReceipt"] = measured["fileSha256"] == take["sha256"] and list(image.size) == [take["width"], take["height"]]
                if data["case"].startswith("boxes-") or data["case"] == "fixed-seed-repeat":
                    measured["boxes"] = [shape_measurement(pixels, color, element["bbox"]) for color, element in zip(("red", "blue"), data["project"]["elements"])]
                item["takes"].append(measured)
            evidence.append(item)
    for backend in ("ideogram", "moss"):
        by_case = {entry["case"]: entry for entry in evidence if entry["backend"] == backend}
        if "boxes-square" in by_case and "fixed-seed-repeat" in by_case:
            original, repeated = by_case["boxes-square"]["takes"], by_case["fixed-seed-repeat"]["takes"]
            by_case["fixed-seed-repeat"]["pixelsMatchInitial"] = bool(original and repeated) and original[0]["decodedPixelSha256"] == repeated[0]["decodedPixelSha256"]
        if "negative-control" in by_case and "negative-prompt" in by_case:
            control, negative = by_case["negative-control"]["takes"], by_case["negative-prompt"]["takes"]
            by_case["negative-prompt"]["pixelsMatchNegativeControl"] = bool(control and negative) and control[0]["decodedPixelSha256"] == negative[0]["decodedPixelSha256"]
        for entry in by_case.values():
            hashes = [take["decodedPixelSha256"] for take in entry["takes"]]
            if len(hashes) > 1:
                entry["allBatchVariationsDistinct"] = len(set(hashes)) == len(hashes)
    (root / "pixel-evidence.json").write_text(json.dumps(evidence, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    sections = []
    for name, pair in records.items():
        columns = []
        for backend in ("ideogram", "moss"):
            data = pair.get(backend)
            if data is None:
                columns.append(f"<article><h3>{backend}</h3><p>Pending</p></article>")
                continue
            pictures = "".join(review_picture(root, backend, name, data["project"], take) for take in data.get("project", {}).get("takes", []) if take["path"])
            review = reviews.get(backend, {}).get(name, {})
            columns.append(f'<article><h3>{backend}</h3><p>{html.escape(data["status"])} · {data.get("elapsedSeconds", 0):.1f}s · visual verdict: {html.escape(review.get("verdict", "pending"))}</p><p>{html.escape(review.get("notes", "Awaiting visual inspection."))}</p>{pictures}<details><summary>Exact receipts</summary><pre>{html.escape(json.dumps(data, ensure_ascii=False, indent=2))}</pre></details></article>')
        criteria = next(iter(pair.values())).get("reviewCriteria", [])
        sections.append(f'<section><h2>{html.escape(name)}</h2><ul>{"".join(f"<li>{html.escape(c)}</li>" for c in criteria)}</ul><div class="pair">{"".join(columns)}</div></section>')
    page = '''<!doctype html><meta charset="utf-8"><title>Kestrel image comparison</title><style>body{font:16px system-ui;background:#eee;color:#222;margin:2rem}h1{margin-bottom:.3em}.pair{display:grid;grid-template-columns:1fr 1fr;gap:1rem}article{background:white;padding:1rem;min-width:0}img{display:block;max-width:100%;max-height:70vh;object-fit:contain}.figure{position:relative;display:inline-block;max-width:100%}.overlay{display:none;position:absolute;inset:0;width:100%;height:100%;pointer-events:none}.overlay rect{fill:none;stroke:#a400bd;stroke-width:3}.overlay text{fill:#a400bd;font:24px system-ui;paint-order:stroke;stroke:white;stroke-width:2}body:has(#boxes:checked) .overlay{display:block}pre{white-space:pre-wrap;overflow-wrap:anywhere}section{margin:3rem 0}details{margin:1rem 0}</style><h1>Kestrel image comparison</h1><p>Native producer pipeline. Click images for full-resolution originals. Generation success is separate from visual quality. Quantitative color-box measurements are in <a href="pixel-evidence.json">pixel-evidence.json</a>.</p><label><input id="boxes" type="checkbox"> Overlay requested placement boxes (review guides, not generated pixels)</label>'''
    (root / "review.html").write_text(page + "".join(sections), encoding="utf-8")
    pending = sum(item["qualityVerdict"] == "pending" for item in evidence)
    print(f"Measured {len(evidence)} case/model records; {pending} await visual review. {root / 'review.html'}")


if __name__ == "__main__":
    main()
