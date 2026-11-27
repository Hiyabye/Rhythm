#!/usr/bin/env python3
"""The OCR step of the collector workflow (#318): reads Bellarmine dormitory
weekly menu images that the collector fetched and writes one result per image.

Usage: meal_ocr.py <folder>

The collector writes `<sha>.json` jobs with their `<sha>.jpg|png` image into the
folder; this writes `<sha>.result.json` next to each. A week is accepted only
when the table's ruling lines, its banner dates and all seven column headers
are read independently and agree with the job's week; anything else is
`rejected` with a short reason. Text is never moved between days, and a merged
or blank cell stays `unknown`. The collector validates every result again
before publishing it (crates/collector/src/meals.rs).

This step runs without secrets or network access beyond the model download,
and it never writes outside the folder except to a temporary crop directory.

Ported from Hiyabye/SogangFriends `scripts/meal-ocr/pipeline.py` by its author.
"""
from datetime import date, timedelta
import hashlib
import json
import math
import os
from pathlib import Path
import re
import sys
import tempfile
import unicodedata

# Must equal `meals::OCR_VERSION` in the collector, which only reads results
# of its own version. Bump both with any change to what a result means.
VERSION = "meal-ocr-1"
WEEKDAYS = "월화수목금토일"
MAX_IMAGE_BYTES = 5 * 1024 * 1024
MAX_PIXELS = 20_000_000
MAX_JOBS = 8
DAY_FIELDS = ("breakfast_korean", "breakfast_simple", "cup_rice", "dinner")
SHARED_FIELDS = ("breakfast_common", "drinks")
# The reviewed weight files (bytes, SHA-256) of the two PaddleOCR models; the
# `inference.pdiparams` digests are the ones SogangFriends reviewed.
MODELS = {
    "PP-OCRv5_server_det": {
        "inference.pdiparams": "183146fe9d9910352f68482f623bcbbb9fa7b9e8fa1463b9ad288cef00524d2d",
        "inference.json": "af5876933d8806a1b50d895867e0781e135cd92ff37381992828fc8d1b842d28",
        "inference.yml": "28fb721efc3634fc8aa677e474b9602cb815a91cf569ef357a7a553d7b3ce685",
    },
    "korean_PP-OCRv5_mobile_rec": {
        "inference.pdiparams": "cac3e5f12cf04aaa77f6a5bc704e4e736ef2908476551891d84b41b4e9090462",
        "inference.json": "562404e3c590c50c93778d5f0a94df21b47b5ab8f3ea6d47c7f8a7930c3bc844",
        "inference.yml": "f757fa1c40e99edcf27e9cce879b93eb2a51fa46f5ef39095689b8c37dd75998",
    },
}
JOB_NAME = re.compile(r"[0-9a-f]{64}")
CLOCK = re.compile(r"(?<!\d)(?:[01]?\d|2[0-3]):[0-5]\d(?!\d)")
# Hangul compatibility jamo never appear in a menu; a lone one is a fragment
# of a sticker or decoration (the 직화 sticker of 2026-11-23 left a `ㄷ`).
JAMO = re.compile(r"[ㄱ-ㆎ]")


class Rejected(ValueError):
    """A deliberately bounded reason code, never an upstream message."""


def normalize_text(text):
    return "".join(unicodedata.normalize("NFC", text).split())


def parse_date(value):
    if not isinstance(value, str):
        raise ValueError("date must be YYYY-MM-DD")
    result = date.fromisoformat(value)
    if result.isoformat() != value:
        raise ValueError("date must be YYYY-MM-DD")
    return result


def groups(values, distance):
    output = []
    for value in values:
        value = int(value)
        if not output or value - output[-1][-1] > distance:
            output.append([value])
        else:
            output[-1].append(value)
    return output


def detect_geometry(image):
    """Recognize only the reviewed eleven-rule/seven-column table; fail closed."""
    import cv2
    import numpy as np
    width, height = image.size
    if width < 700 or height < 500 or width * height > MAX_PIXELS:
        raise Rejected("unsupported_image_dimensions")
    rgb = np.asarray(image.convert("RGB"))
    red, green, blue = rgb[:, :, 0], rgb[:, :, 1], rgb[:, :, 2]
    # Orange/red rules vary by week. Text does not survive the long-line opening.
    warm = (red > 170) & (green < 190) & (blue < 130)
    dark = (rgb.max(axis=2) < 120).astype("uint8") * 255
    # Lighter cell borders cross the black rules, leaving 1-2 px gaps; on a
    # small inline PNG (1088x787, 2026-11-30) they cut a rule that a merged
    # cell already shortens into pieces under the kernel below. Close such
    # gaps in the dark mask only: closing the orange header would fill its
    # white text into one solid band. Words stay far shorter than 35%.
    gap = max(3, round(width * .003))
    dark = cv2.morphologyEx(dark, cv2.MORPH_CLOSE, np.ones((1, gap), np.uint8))
    mask = ((warm * 255) | dark).astype("uint8")
    # Highlight borders can shift a row rule by a few pixels mid-table. Keep
    # long segments before grouping nearby rows; require >55% total coverage.
    line = cv2.morphologyEx(mask, cv2.MORPH_OPEN, np.ones((1, round(width * .35)), np.uint8))
    ys = [round(sum(row) / len(row)) for row in groups(
        np.flatnonzero((line > 0).sum(axis=1) > width * .55), max(2, round(height * .004)))]
    if len(ys) != 11 or not .07 < ys[0] / height < .2 or not .8 < ys[-1] / height < .96:
        raise Rejected("unsupported_horizontal_grid")
    # Header, Korean menu, calories, simple menu, calories, shared row, cup rice,
    # dinner, calories, drinks. Sizes enforce the supported structure, not dates.
    low = [.025, .12, .008, .08, .008, .008, .018, .12, .008, .008]
    high = [.08, .3, .04, .23, .04, .04, .11, .3, .04, .04]
    if any(not a < (right - left) / height < b for left, right, a, b in zip(ys, ys[1:], low, high)):
        raise Rejected("unsupported_row_structure")

    def verticals(top, bottom):
        if bottom - top < 5:
            raise Rejected("empty_table_row")
        values = np.flatnonzero((mask[top:bottom] > 0).mean(axis=0) > .8)
        return [round(sum(col) / len(col)) for col in groups(values, max(2, round(width * .002)))]

    columns = verticals(ys[1] + 8, ys[2] - 8)
    if len(columns) != 10:
        raise Rejected("unsupported_vertical_grid")
    xs = columns[-8:]
    widths = [right - left for left, right in zip(xs, xs[1:])]
    if (not .08 < xs[0] / width < .2 or not .85 < xs[-1] / width < .98
            or min(widths) < width * .08 or max(widths) / min(widths) > 1.3):
        raise Rejected("unsupported_column_structure")
    # A date heading alone cannot establish menu alignment: recheck the menu
    # rules. A merged cell cannot be assigned to one date, so BOTH adjacent
    # cells become unknown instead of slicing its text into guessed days.
    unknown_cells = {}
    for field, top, bottom in (("breakfast_simple", ys[3] + 8, ys[4] - 8),
                               ("cup_rice", ys[6] + 8, ys[7] - 8),
                               ("dinner", ys[7] + 8, ys[8] - 8)):
        observed = verticals(top, bottom)
        matches = [any(abs(x - candidate) <= width * .003 for candidate in observed) for x in xs]
        unexpected = [x for x in observed if xs[0] <= x <= xs[-1]
                      and not any(abs(x - candidate) <= width * .003 for candidate in xs)]
        if not matches[0] or not matches[-1] or unexpected:
            raise Rejected("misaligned_column_rules")
        unknown_cells[field] = sorted({cell for boundary in range(1, 7) if not matches[boundary]
                                       for cell in (boundary - 1, boundary)})
    inset = max(3, round(min(width, height) * .002))
    spans = {"day_header": (ys[0], ys[1]), "breakfast_korean": (ys[1], ys[2]),
             "breakfast_simple": (ys[3], ys[4]), "breakfast_common": (ys[5], ys[6]),
             "cup_rice": (ys[6], ys[7]), "dinner": (ys[7], ys[8]), "drinks": (ys[9], ys[10])}
    # Centered above the grid: excludes the contacts on the right and the logo
    # on the left.
    crops = {"banner": [round(width * .4), round(height * .015), round(width * .64), ys[0] - inset]}
    for name, (top, bottom) in spans.items():
        if name in SHARED_FIELDS:
            crops[name] = [xs[0] + inset, top + inset, xs[-1] - inset, bottom - inset]
        else:
            for index in range(7):
                crops[f"{name}_{index}"] = [xs[index] + inset, top + inset, xs[index + 1] - inset, bottom - inset]
    if any(left >= right or top >= bottom for left, top, right, bottom in crops.values()):
        raise Rejected("empty_crop")
    return {"rules": ys, "day_boundaries": xs, "inset": inset, "unknown_cells": unknown_cells, "crops": crops}


def rows(detections):
    """Groups detections into printed lines, top to bottom, each left to right."""
    found = []
    for item in sorted(detections, key=lambda item: item["top"]):
        for row in found:
            overlap = min(item["bottom"], row["bottom"]) - max(item["top"], row["top"])
            # Same-line fragments can differ by a pixel in Y.
            if overlap >= 0.5 * min(item["bottom"] - item["top"], row["bottom"] - row["top"]):
                row["items"].append(item)
                break
        else:
            found.append({"top": item["top"], "bottom": item["bottom"], "items": [item]})
    return [" ".join(item["text"] for item in sorted(row["items"], key=lambda item: item["left"]))
            for row in found]


def detection(box, text, score):
    points = box.tolist() if hasattr(box, "tolist") else box
    if not isinstance(points, (list, tuple)) or len(points) != 4:
        raise ValueError("OCR returned an invalid quadrilateral")
    xs, ys = [], []
    for point in points:
        x, y = (float(value) for value in point)
        if not (math.isfinite(x) and math.isfinite(y)):
            raise ValueError("OCR returned invalid coordinates")
        xs.append(x)
        ys.append(y)
    score = float(score)
    if not isinstance(text, str) or not math.isfinite(score) or not 0 <= score <= 1:
        raise ValueError("OCR returned invalid text or confidence")
    return {"text": text, "left": min(xs), "top": min(ys), "bottom": max(ys)}


def paddle_detections(results):
    detections = []
    for result in results:
        data = result.json
        if callable(data):
            data = data()
        if isinstance(data, str):
            data = json.loads(data)
        data = data.get("res", data)
        texts, scores, boxes = data["rec_texts"], data["rec_scores"], data["rec_polys"]
        if not len(texts) == len(scores) == len(boxes):
            raise ValueError("PaddleOCR result array lengths disagree")
        detections.extend(detection(box, text, score) for box, text, score in zip(boxes, texts, scores))
    return detections


def bounded(lines):
    if len(lines) > 40 or any(len(line) > 500 for line in lines):
        raise Rejected("invalid_ocr_lines")
    return lines


def identity_lines(lines):
    if not 1 <= len(lines) <= 12 or any(not line.strip() or len(line) > 150 for line in lines):
        raise Rejected("invalid_identity_observations")
    return lines


def week_of(job):
    start = parse_date(job["week"])
    published = parse_date(job["published"])
    if start.weekday() != 0 or abs((start - published).days) > 45:
        raise Rejected("invalid_job_week")
    return start


def observe_identity(banner, headers, start):
    """The week's seven dates, only when the banner and every header agree."""
    end = start + timedelta(days=6)
    text = normalize_text("".join(identity_lines(banner)))
    # Dot/comma punctuation substitutions do not change observed numeric dates.
    pattern = r"(?<!\d)(\d{4})[.,/-](\d{1,2})[.,/-](\d{1,2})[~～–-](?:(\d{4})[.,/-])?(\d{1,2})[.,/-](\d{1,2})(?!\d)"
    matches = list(re.finditer(pattern, text))
    if len(matches) != 1:
        raise Rejected("unreadable_banner_dates")
    match = matches[0]
    year, month, day = map(int, match.group(1, 2, 3))
    end_month, end_day = map(int, match.group(5, 6))
    end_year = int(match.group(4)) if match.group(4) else year + int(end_month < month)
    try:
        observed = (date(year, month, day), date(end_year, end_month, end_day))
    except ValueError:
        raise Rejected("invalid_banner_dates") from None
    if observed != (start, end):
        raise Rejected("conflicting_banner_dates")
    if len(headers) != 7:
        raise Rejected("missing_column_headers")
    dates = []
    for index, lines in enumerate(headers):
        text = normalize_text("".join(identity_lines(lines)))
        observed_date = list(re.finditer(r"(?<!\d)(\d{1,2})월(\d{1,2})일?(?!\d)", text))
        weekdays = re.findall(r"([월화수목금토일])요일", text)
        candidate = start + timedelta(days=index)
        if len(observed_date) != 1 or len(weekdays) != 1:
            raise Rejected("unreadable_column_date")
        mm, dd = map(int, observed_date[0].group(1, 2))
        years = re.findall(r"(\d{4})년", text)
        if ((mm, dd) != (candidate.month, candidate.day) or weekdays[0] != WEEKDAYS[candidate.weekday()]
                or len(years) > 1 or years and int(years[0]) != candidate.year):
            raise Rejected("conflicting_column_date")
        # Usable only after the independently read header agrees with the
        # independently read banner; array position alone never authorizes it.
        dates.append(candidate)
    return dates


def clean(line):
    line = JAMO.sub("", unicodedata.normalize("NFC", line))
    # The menu spaces `&` and `/` inconsistently (`A & B`, `A&B`, `2종/ 우유`).
    line = re.sub(r"\s*([&/])\s*", r"\1", line)
    return " ".join(line.split()).strip("&/ ")


def items_of(lines):
    """Menu items from printed lines: a line that starts or follows `&`
    continues the item before it (a two-line `치즈 함박스테이크 / & 데미그라소스`)."""
    items = []
    open_ended = False
    for line in lines:
        text = clean(line)
        if not text:
            continue
        if items and (open_ended or line.lstrip().startswith("&")):
            items[-1] = clean(items[-1] + "&" + text)
        else:
            items.append(text)
        open_ended = line.rstrip().endswith("&")
    return items


def offering(lines):
    """One cell: `available` with items (and a printed time), `closed`, or
    `unknown` when nothing usable was read."""
    bounded(lines)
    # Recommendation badges sit in otherwise empty cells; they are decoration.
    lines = [line for line in lines if line.strip()
             and not line.strip().startswith("#")
             and not re.fullmatch(r"(?:강력추(?:천)?|추천|HIT|BEST|NEW)", normalize_text(line), re.IGNORECASE)]
    unknown = {"status": "unknown", "items": [], "time": None}
    if not lines:
        return unknown
    normalized = normalize_text(" ".join(lines))
    if any(token in normalized for token in ("미운영", "운영안함", "운영없음", "휴무", "식사없음")):
        return {"status": "closed", "items": [], "time": None}
    times = CLOCK.findall(" ".join(lines))
    items = items_of([CLOCK.sub("", line) for line in lines])
    if not items or len(items) > 20 or any(len(item) > 150 for item in items):
        return unknown
    time = times[0] if len(times) == 1 else None
    if time is not None and len(time) == 4:
        time = "0" + time
    return {"status": "available", "items": items, "time": time}


def extract_week(image_path, job, infer):
    """The week's result for one image. Raises `Rejected` for an image this
    pipeline cannot read with certainty."""
    from PIL import Image, UnidentifiedImageError
    start = week_of(job)
    try:
        opened = Image.open(image_path)
    except UnidentifiedImageError:
        raise Rejected("unsupported_image_format") from None
    except Image.DecompressionBombError:
        raise Rejected("unsupported_image_dimensions") from None
    with opened as image:
        if image.format not in ("PNG", "JPEG") or getattr(image, "n_frames", 1) != 1:
            raise Rejected("unsupported_image_format")
        if image.width * image.height > MAX_PIXELS:
            raise Rejected("unsupported_image_dimensions")
        width, height = image.size
        try:
            image.load()
        except (OSError, SyntaxError, ValueError):
            # The header parsed but the pixel data is cut short or corrupt
            # (Pillow reports a broken PNG chunk as SyntaxError).
            raise Rejected("unsupported_image_format") from None
        geometry = detect_geometry(image)
        image = image.convert("RGB")
        with tempfile.TemporaryDirectory() as directory:
            def read(crop):
                path = Path(directory) / f"{crop}.png"
                image.crop(geometry["crops"][crop]).save(path)
                return bounded(rows(infer(path)))

            banner = read("banner")
            headers = [read(f"day_header_{index}") for index in range(7)]
            dates = observe_identity(banner, headers, start)
            days = []
            for index, day in enumerate(dates):
                cells = {"day": day.isoformat()}
                for field in DAY_FIELDS:
                    if index in geometry["unknown_cells"].get(field, []):
                        cells[field] = {"status": "unknown", "items": [], "time": None}
                    else:
                        cells[field] = offering(read(f"{field}_{index}"))
                days.append(cells)
            shared = {field: offering(read(field)) for field in SHARED_FIELDS}
    return {"image_size": [width, height], "banner": banner, "headers": headers, "days": days, **shared}


def load_engine():
    from paddleocr import PaddleOCR
    engine = PaddleOCR(text_detection_model_name="PP-OCRv5_server_det",
                       text_recognition_model_name="korean_PP-OCRv5_mobile_rec", device="cpu",
                       use_doc_orientation_classify=False, use_doc_unwarping=False,
                       use_textline_orientation=False)
    return lambda path: paddle_detections(engine.predict(str(path)))


def check_weights(home):
    """Aborts the step (`SystemExit`) unless every reviewed weight file exists
    and matches its digest. PaddleX downloads the weights while the engine is
    constructed, so the paths exist only after that: the check runs right after
    construction and before any image is decoded or inferred. A missing or
    different file is fatal, never a per-image failure."""
    for model, files in MODELS.items():
        for name, expected in files.items():
            digest = hashlib.sha256()
            try:
                with (home / "official_models" / model / name).open("rb") as handle:
                    for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                        digest.update(chunk)
            except OSError:
                raise SystemExit(f"{model}/{name} is missing or unreadable") from None
            if digest.hexdigest() != expected:
                raise SystemExit(f"{model}/{name} is not the reviewed model file")


def read_job(folder, path):
    if path.stat().st_size > 4096:
        raise ValueError("job too large")
    job = json.loads(path.read_text(encoding="utf-8"))
    sha = path.stem
    if (not isinstance(job, dict) or set(job) != {"version", "image", "image_sha256", "week", "published"}
            or job["version"] != VERSION or job["image_sha256"] != sha
            or job["image"] not in (f"{sha}.jpg", f"{sha}.png")):
        raise ValueError("invalid job")
    image = folder / job["image"]
    if image.stat().st_size > MAX_IMAGE_BYTES or hashlib.sha256(image.read_bytes()).hexdigest() != sha:
        raise ValueError("image does not match its job")
    return job, image


def run(folder, engine_factory=load_engine, weights_home=None):
    """Writes a result for every job; returns how many could not be read for a
    reason other than the image. Those get no result, so the collector offers
    them again next run and reports them meanwhile."""
    jobs = sorted(path for path in folder.glob("*.json") if JOB_NAME.fullmatch(path.stem))
    if len(jobs) > MAX_JOBS:
        raise SystemExit(f"more than {MAX_JOBS} jobs")
    infer = None
    checked = False
    failed = 0
    for path in jobs:
        try:
            job, image = read_job(folder, path)
        except (ValueError, OSError) as error:
            # One unusable job must not cost the other weeks their results.
            print(f"{path.stem}: job not read ({type(error).__name__})", file=sys.stderr)
            failed += 1
            continue
        result = {"version": VERSION, "image_sha256": job["image_sha256"]}
        try:
            if infer is None:
                infer = engine_factory()
        except (ValueError, KeyError, OSError, RuntimeError) as error:
            print(f"{path.stem}: OCR failed ({type(error).__name__})", file=sys.stderr)
            failed += 1
            continue
        if weights_home is not None and not checked:
            check_weights(weights_home)
            checked = True
        try:
            result.update(status="ok", reason=None, **extract_week(image, job, infer))
        except Rejected as error:
            result.update(status="rejected", reason=str(error))
        except Exception as error:
            # An engine or system failure says nothing about the image: no
            # result, so the collector offers it again. Third-party code
            # raises its own types (`cv2.error`), and none of them may stop
            # the remaining jobs. Never print the error body, which may quote
            # third-party output.
            print(f"{path.stem}: OCR failed ({type(error).__name__})", file=sys.stderr)
            failed += 1
            continue
        with (folder / f"{path.stem}.result.json").open("x", encoding="utf-8") as handle:
            json.dump(result, handle, ensure_ascii=False, indent=1)
            handle.write("\n")
        print(f"{path.stem}: {result['status']}" + (f" ({result['reason']})" if result["reason"] else ""))
    return failed


def main(argv):
    if len(argv) != 2:
        raise SystemExit(__doc__.split("\n\n")[1])
    home = Path(os.environ.get("PADDLE_PDX_CACHE_HOME") or Path.home() / ".paddlex")
    failed = run(Path(argv[1]), weights_home=home)
    # Exit 0 even then: the other results still go to the collector.
    print(f"{failed} images without a result")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
