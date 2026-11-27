"""Tests of meal_ocr.py (#318). Engines are injected: these never load a model
or touch the network. Geometry and whole-week tests need Pillow, OpenCV and
NumPy from requirements.lock and skip without them; the stdlib tests always
run. `MEAL_OCR_REAL=1` additionally reads the committed real week with
PaddleOCR (downloads the reviewed weights; never part of CI)."""
from datetime import date, timedelta
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

HERE = Path(__file__).parent
sys.path.insert(0, str(HERE))
import meal_ocr  # noqa: E402

try:
    from PIL import Image, ImageDraw
    import cv2  # noqa: F401
except ImportError:
    Image = None


def job(start="2026-10-12"):
    first = date.fromisoformat(start)
    return {"week": start, "published": (first - timedelta(days=4)).isoformat()}


def identity(start="2026-10-12"):
    first = date.fromisoformat(start)
    last = first + timedelta(days=6)
    banner = [f"{first:%Y.%m.%d}~{last:%Y.%m.%d}"]
    headers = [[f"{(first + timedelta(days=i)).month}월 {(first + timedelta(days=i)).day}일",
                f"{meal_ocr.WEEKDAYS[i]}요일"] for i in range(7)]
    return banner, headers


def start(value="2026-10-12"):
    return date.fromisoformat(value)


def grid(dx=0):
    picture = Image.new("RGB", (2000, 1000), "white")
    draw = ImageDraw.Draw(picture)
    ys = [125, 175, 380, 406, 560, 586, 612, 670, 856, 882, 910]
    xs = [100, 180, 300, 520, 740, 960, 1180, 1400, 1620, 1840]
    for y in ys:
        draw.line((xs[0] + dx, y, xs[-1] + dx, y), fill="black", width=3)
    for x in xs:
        draw.line((x + dx, 175, x + dx, 586), fill="black", width=3)
        draw.line((x + dx, 612, x + dx, 910), fill="black", width=3)
    return picture


def detections(lines):
    """One detection per line, stacked vertically."""
    return [{"text": text, "left": 10, "top": i * 20 + 2, "bottom": i * 20 + 16} for i, text in enumerate(lines)]


class IdentityTests(unittest.TestCase):
    def test_observed_banner_and_every_weekday_required(self):
        banner, headers = identity()
        dates = meal_ocr.observe_identity(banner, headers, start())
        self.assertEqual([day.isoformat() for day in dates], [f"2026-10-{i}" for i in range(12, 19)])
        headers[1][1] = "월요일"
        with self.assertRaisesRegex(meal_ocr.Rejected, "conflicting_column_date"):
            meal_ocr.observe_identity(banner, headers, start())

    def test_observation_bounds(self):
        banner, headers = identity()
        for invalid in (banner + ["extra"] * 12, [banner[0], ""], [banner[0], "x" * 151]):
            with self.assertRaises(meal_ocr.Rejected):
                meal_ocr.observe_identity(invalid, headers, start())
        headers[0] += ["extra"] * 12
        with self.assertRaises(meal_ocr.Rejected):
            meal_ocr.observe_identity(banner, headers, start())

    def test_month_and_year_rollover(self):
        for first in ("2026-09-28", "2026-12-28"):
            banner, headers = identity(first)
            result = meal_ocr.observe_identity(banner, headers, start(first))
            self.assertEqual(result[-1], start(first) + timedelta(days=6))
        _, headers = identity("2026-12-28")
        self.assertEqual(meal_ocr.observe_identity(["2026.12.28~01.03"], headers, start("2026-12-28"))[-1].year, 2027)

    def test_rejects_job_banner_conflict_and_swapped_or_missing_headers(self):
        _, headers = identity("2026-09-14")
        with self.assertRaisesRegex(meal_ocr.Rejected, "conflicting_banner_dates"):
            meal_ocr.observe_identity(["2026.09.07~2026.09.13"], headers, start("2026-09-14"))
        banner, headers = identity()
        headers[0], headers[1] = headers[1], headers[0]
        with self.assertRaises(meal_ocr.Rejected):
            meal_ocr.observe_identity(banner, headers, start())
        with self.assertRaises(meal_ocr.Rejected):
            meal_ocr.observe_identity(banner, headers[:6], start())
        with self.assertRaises(meal_ocr.Rejected):
            meal_ocr.observe_identity([], identity()[1], start())

    def test_digits_must_not_be_substrings_of_longer_numbers(self):
        banner, headers = identity()
        for text in ("12026.10.12~10.18", "2026.10.12~10.181"):
            with self.assertRaises(meal_ocr.Rejected):
                meal_ocr.observe_identity([text], headers, start())
        headers[0][0] = "110월 12일"
        with self.assertRaises(meal_ocr.Rejected):
            meal_ocr.observe_identity(banner, headers, start())

    def test_conflicting_header_year_rejected(self):
        banner, headers = identity()
        headers[0].insert(0, "2025년")
        with self.assertRaises(meal_ocr.Rejected):
            meal_ocr.observe_identity(banner, headers, start())

    def test_sticker_text_after_the_weekday_still_parses(self):
        # 2026-11-23: the 직화 sticker over Monday's header read as `월요일직`.
        banner, headers = identity("2026-11-23")
        headers[0] = ["11월 23일", "월요일직"]
        self.assertEqual(meal_ocr.observe_identity(banner, headers, start("2026-11-23"))[0].day, 23)

    def test_job_week_must_be_a_monday_near_publication(self):
        self.assertEqual(meal_ocr.week_of(job()), start())
        for bad in ({"week": "2026-10-13", "published": "2026-10-08"},
                    {"week": "2026-10-12", "published": "2026-06-01"}):
            with self.assertRaisesRegex(meal_ocr.Rejected, "invalid_job_week"):
                meal_ocr.week_of(bad)


class OfferingTests(unittest.TestCase):
    def test_typo_is_kept_blank_is_unknown_closure_is_closed(self):
        self.assertEqual(meal_ocr.offering(["계란짐"])["items"], ["계란짐"])
        self.assertEqual(meal_ocr.offering([])["status"], "unknown")
        self.assertEqual(meal_ocr.offering(["토요일석식미운영"]),
                         {"status": "closed", "items": [], "time": None})

    def test_badges_alone_are_not_items(self):
        for lines in (["강력추"], ["강력추천"], ["HIT"], ["#수제베이커리#"], ["#Self 베이커리"]):
            self.assertEqual(meal_ocr.offering(lines)["status"], "unknown")
        self.assertEqual(meal_ocr.offering(["#수제베이커리#", "카라멜애플파이"])["items"], ["카라멜애플파이"])

    def test_printed_time_is_kept_apart_from_items(self):
        self.assertEqual(meal_ocr.offering(["11:40", "돈까스덮밥"]),
                         {"status": "available", "items": ["돈까스덮밥"], "time": "11:40"})
        self.assertEqual(meal_ocr.offering(["9:30", "덮밥"])["time"], "09:30")
        self.assertIsNone(meal_ocr.offering(["쌀밥"])["time"])
        self.assertEqual(meal_ocr.offering(["11:40"])["status"], "unknown")
        self.assertIsNone(meal_ocr.offering(["11:40", "12:00", "쌀밥"])["time"])

    def test_items_continue_across_ampersands_and_lose_stray_jamo(self):
        # 2026-11-23 Wednesday dinner (two printed lines) and Monday breakfast
        # (the sticker added a `ㄷ`).
        self.assertEqual(meal_ocr.offering(["치즈 함박스테이크", "& 데미그라소스", "핫바"])["items"],
                         ["치즈 함박스테이크&데미그라소스", "핫바"])
        self.assertEqual(meal_ocr.offering(["돼지불백전골&ㄷ 당면사리"])["items"], ["돼지불백전골&당면사리"])
        self.assertEqual(meal_ocr.offering(["돼지불백전골 &", "당면사리"])["items"], ["돼지불백전골&당면사리"])
        self.assertEqual(meal_ocr.offering(["씨리얼 2종/ 우유"])["items"], ["씨리얼 2종/우유"])

    def test_rows_join_fragments_of_one_printed_line(self):
        found = [{"text": "함박스테이크", "left": 60, "top": 1, "bottom": 15},
                 {"text": "치즈", "left": 10, "top": 0, "bottom": 14},
                 {"text": "& 데미그라소스", "left": 20, "top": 20, "bottom": 34}]
        self.assertEqual(meal_ocr.rows(found), ["치즈 함박스테이크", "& 데미그라소스"])


class JobTests(unittest.TestCase):
    def write(self, folder, data=b"not an image", **changes):
        sha = hashlib.sha256(data).hexdigest()
        (folder / f"{sha}.jpg").write_bytes(data)
        body = {"version": meal_ocr.VERSION, "image": f"{sha}.jpg", "image_sha256": sha, **job(), **changes}
        (folder / f"{sha}.json").write_text(json.dumps(body))
        return sha

    def test_job_must_match_its_image_and_version(self):
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            sha = self.write(folder)
            meal_ocr.read_job(folder, folder / f"{sha}.json")
            (folder / f"{sha}.jpg").write_bytes(b"changed")
            with self.assertRaises(ValueError):
                meal_ocr.read_job(folder, folder / f"{sha}.json")
            other = self.write(folder, b"other", version="meal-ocr-0")
            with self.assertRaises(ValueError):
                meal_ocr.read_job(folder, folder / f"{other}.json")
            third = self.write(folder, b"third", image="../escape.jpg")
            with self.assertRaises(ValueError):
                meal_ocr.read_job(folder, folder / f"{third}.json")

    def test_unreadable_image_is_rejected_and_engine_failure_writes_nothing(self):
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            sha = self.write(folder)

            def broken():
                raise RuntimeError("must not leak this raw provider error")

            self.assertEqual(meal_ocr.run(folder, broken), 1)
            self.assertFalse((folder / f"{sha}.result.json").exists())
            if Image is None:
                return
            # Pillow cannot open the bytes: the image itself is rejected.
            self.assertEqual(meal_ocr.run(folder, lambda: (lambda path: [])), 0)
            result = json.loads((folder / f"{sha}.result.json").read_text())
            self.assertEqual(result, {"version": meal_ocr.VERSION, "image_sha256": sha,
                                      "status": "rejected", "reason": "unsupported_image_format"})

    def test_missing_or_changed_weights_abort_the_step(self):
        with tempfile.TemporaryDirectory() as directory, tempfile.TemporaryDirectory() as home:
            folder = Path(directory)
            sha = self.write(folder)

            def engine():
                return lambda path: []

            # No weight files at all: fatal, not a per-image failure.
            with self.assertRaises(SystemExit):
                meal_ocr.run(folder, engine, weights_home=Path(home))
            self.assertFalse((folder / f"{sha}.result.json").exists())
            # Present but different bytes: fatal too.
            for model, files in meal_ocr.MODELS.items():
                target = Path(home) / "official_models" / model
                target.mkdir(parents=True)
                for name in files:
                    (target / name).write_bytes(b"other")
            with self.assertRaises(SystemExit):
                meal_ocr.run(folder, engine, weights_home=Path(home))
            self.assertFalse((folder / f"{sha}.result.json").exists())


@unittest.skipIf(Image is None, "Pillow and OpenCV from requirements.lock are needed")
class GeometryTests(unittest.TestCase):
    def test_detects_shifted_columns_and_insets_every_crop(self):
        first = meal_ocr.detect_geometry(grid())
        second = meal_ocr.detect_geometry(grid(35))
        self.assertEqual(second["day_boundaries"], [x + 35 for x in first["day_boundaries"]])
        for i in range(7):
            x0, _, x1, _ = first["crops"][f"breakfast_korean_{i}"]
            self.assertGreater(x0, first["day_boundaries"][i])
            self.assertLess(x1, first["day_boundaries"][i + 1])
        self.assertEqual(first["crops"]["breakfast_common"][0], first["day_boundaries"][0] + first["inset"])

    def test_missing_horizontal_rule_is_not_guessed(self):
        image = grid()
        ImageDraw.Draw(image).rectangle((0, 377, 1999, 383), fill="white")
        with self.assertRaisesRegex(meal_ocr.Rejected, "unsupported_horizontal_grid"):
            meal_ocr.detect_geometry(image)

    def test_a_rule_cut_by_a_merged_cell_and_light_borders_is_still_found(self):
        # The 2026-11-30 inline PNG: Saturday's dinner merges into its calorie
        # row, and light-gray borders leave a one-pixel gap at every column.
        picture = grid()
        draw = ImageDraw.Draw(picture)
        draw.rectangle((1406, 854, 1614, 858), fill="white")
        for x in [180, 300, 520, 740, 960, 1180]:
            draw.line((x, 854, x, 858), fill=(200, 200, 200), width=1)
        geometry = meal_ocr.detect_geometry(picture)
        self.assertEqual(len(geometry["rules"]), 11)
        self.assertLessEqual(abs(geometry["rules"][8] - 856), 3)
        # Pieces that short with wider gaps are still no rule.
        for x in range(150, 1400, 100):
            draw.rectangle((x, 853, x + 12, 859), fill="white")
        with self.assertRaisesRegex(meal_ocr.Rejected, "unsupported_horizontal_grid"):
            meal_ocr.detect_geometry(picture)

    def test_real_weeks_have_eleven_rules_and_seven_columns(self):
        # No engine: the table geometry of both image kinds the board posts,
        # a 3508x2480 JPEG and a 1088x787 inline PNG (Saturday dinner merged
        # into its calorie row, light-gray cell borders).
        for name in ("week-2026-11-23.jpg", "week-2026-11-30.png"):
            with self.subTest(name=name), Image.open(HERE / "fixtures" / name) as image:
                geometry = meal_ocr.detect_geometry(image)
                self.assertEqual(len(geometry["rules"]), 11)
                self.assertEqual(len(geometry["day_boundaries"]), 8)
                self.assertTrue(all(not cells for cells in geometry["unknown_cells"].values()))

    def test_merged_cells_are_unknown(self):
        image = grid()
        ImageDraw.Draw(image).rectangle((957, 616, 963, 666), fill="white")
        self.assertEqual(meal_ocr.detect_geometry(image)["unknown_cells"]["cup_rice"], [2, 3])

    def test_shifted_cup_grid_rejected(self):
        image = grid()
        draw = ImageDraw.Draw(image)
        draw.rectangle((0, 616, 1999, 666), fill="white")
        for x in [100, 180, 330, 550, 770, 990, 1210, 1430, 1650, 1870]:
            draw.line((x, 616, x, 666), fill="black", width=3)
        with self.assertRaisesRegex(meal_ocr.Rejected, "misaligned_column_rules"):
            meal_ocr.detect_geometry(image)

    def fake(self, wrong_banner=False, read=None):
        banner, headers = identity()

        def infer(path):
            stem = path.stem
            if read is not None:
                read.append(stem)
            if stem == "banner":
                return detections(["2026.10.05~10.11"] if wrong_banner else banner)
            if stem.startswith("day_header_"):
                return detections(headers[int(stem.rsplit("_", 1)[1])])
            if stem.startswith("cup_rice_"):
                return []
            if stem == "dinner_5":
                return detections(["토요일석식미운영"])
            return detections(["계란짐"])
        return infer

    def extract(self, picture, infer):
        with tempfile.TemporaryDirectory() as directory:
            image = Path(directory) / "source.png"
            picture.save(image)
            return meal_ocr.extract_week(image, job(), infer)

    def test_whole_week(self):
        result = self.extract(grid(), self.fake())
        self.assertEqual(result["image_size"], [2000, 1000])
        self.assertEqual([day["day"] for day in result["days"]], [f"2026-10-{i}" for i in range(12, 19)])
        self.assertEqual(result["days"][0]["breakfast_korean"]["items"], ["계란짐"])
        self.assertEqual(result["days"][0]["cup_rice"]["status"], "unknown")
        self.assertEqual(result["days"][5]["dinner"]["status"], "closed")
        self.assertEqual(result["drinks"]["items"], ["계란짐"])

    def test_wrong_banner_rejects_the_week(self):
        with self.assertRaisesRegex(meal_ocr.Rejected, "conflicting_banner_dates"):
            self.extract(grid(), self.fake(wrong_banner=True))

    def test_merged_cells_are_never_read(self):
        image = grid()
        ImageDraw.Draw(image).rectangle((957, 616, 963, 666), fill="white")
        read = []
        result = self.extract(image, self.fake(read=read))
        self.assertNotIn("cup_rice_2", read)
        self.assertNotIn("cup_rice_3", read)
        self.assertEqual(result["days"][2]["cup_rice"]["status"], "unknown")


@unittest.skipUnless(os.environ.get("MEAL_OCR_REAL") == "1", "MEAL_OCR_REAL=1 reads the real week with PaddleOCR")
class RealWeekTests(unittest.TestCase):
    def test_week_of_2026_11_23(self):
        image = HERE / "fixtures" / "week-2026-11-23.jpg"
        truth = json.loads((HERE / "fixtures" / "week-2026-11-23.truth.json").read_text(encoding="utf-8"))
        result = meal_ocr.extract_week(image, {"week": "2026-11-23", "published": "2026-11-18"},
                                       meal_ocr.load_engine())
        observed = {key: result[key] for key in truth}
        differences = [(key, day) for key in truth for day in range(len(truth[key]) if key == "days" else 1)
                       if (observed[key][day] if key == "days" else observed[key])
                       != (truth[key][day] if key == "days" else truth[key])]
        # The reviewed truth spells `고춧잎무침`; PaddleOCR reads `고줏잎무침`
        # (the known limit: spelling is not checked). Nothing else may differ.
        self.assertEqual(differences, [("days", 0)])
        self.assertEqual(observed["days"][0]["dinner"]["items"][4], "고줏잎무침")


if __name__ == "__main__":
    unittest.main()
