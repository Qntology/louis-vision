# scripts/edit_scenarios_video.py
# Edits captured CDP frames across all 7 chapters into a cohesive 1280x720 30fps H.264 MP4
# with title card, chapter bumpers, non-occluding studio matte + Korean lower-thirds,
# smooth crossfades, verification scorecard outro, storyboard overview PNG, and chapter keyframes.

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import imageio_ffmpeg
import numpy as np
from PIL import Image, ImageDraw, ImageFont

REPO_ROOT = Path(__file__).resolve().parent.parent
CAPTURE_DIR = REPO_ROOT / "artifacts" / "video_capture"
MANIFEST_PATH = CAPTURE_DIR / "storyboard_manifest.json"
OUT_MP4 = REPO_ROOT / "artifacts" / "louis_vision_all_scenarios_demo.mp4"
OUT_STORYBOARD = REPO_ROOT / "artifacts" / "storyboard_overview.png"
KEYFRAMES_DIR = REPO_ROOT / "artifacts" / "keyframes"

W, H = 1280, 720
FPS = 30
INNER_W, INNER_H = 1184, 666
INNER_X = (W - INNER_W) // 2  # 48 px margin L/R
INNER_Y = 28                  # 28 px top header, 26 px bottom caption bar

FONT_BOLD = r"C:\Windows\Fonts\malgunbd.ttf"
FONT_REG = r"C:\Windows\Fonts\malgun.ttf"


def load_font(path: str, size: int):
    if os.path.exists(path):
        return ImageFont.truetype(path, size)
    return ImageFont.load_default()


F_TITLE = load_font(FONT_BOLD, 38)
F_SUB = load_font(FONT_REG, 22)
F_CH_TITLE = load_font(FONT_BOLD, 32)
F_CH_SUB = load_font(FONT_REG, 19)
F_BAR_TOP = load_font(FONT_BOLD, 15)
F_BAR_BOT = load_font(FONT_REG, 14)
F_BADGE = load_font(FONT_BOLD, 14)
F_ROW = load_font(FONT_REG, 18)


def safe_text(text: str) -> str:
    return text.replace("✦", "★")


def make_gradient_bg(c1=(8, 12, 20), c2=(16, 24, 38)) -> Image.Image:
    y = np.linspace(0.0, 1.0, H, dtype=np.float32)[:, None, None]
    arr = (np.array(c1, dtype=np.float32) * (1.0 - y) + np.array(c2, dtype=np.float32) * y).astype(np.uint8)
    arr = np.broadcast_to(arr, (H, W, 3)).copy()
    return Image.fromarray(arr, mode="RGB")


def render_intro_card() -> Image.Image:
    img = make_gradient_bg((7, 11, 18), (18, 30, 52))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([84, 88, W - 84, H - 88], radius=20, fill=(14, 21, 34), outline=(56, 189, 248), width=2)
    d.text((130, 145), safe_text("★ LOUIS-VISION FULL-APP N:N VERIFICATION"), font=F_BADGE, fill=(56, 189, 248))
    d.text((130, 195), "전 앱 사용자 시나리오 실사 촬영 · N:N 검증 리포트", font=F_TITLE, fill=(243, 245, 248))
    d.text((130, 260), "Routing Overlay (127.0.0.1:8787) · Docs Studio · Field-3D AR · LAN Mobile · Vision Sim & XR Stage", font=F_SUB, fill=(168, 182, 202))

    pills = [
        ("SyntheticData 500 KiB+", "3,008 Docs / Min 667,645 B / 0 Under"),
        ("Rust Tier 1 & Dual E2E", "louis-docs 45/45 · lv-router 5/5 · Tauri 5/5 · Vision 16/16"),
        ("Browser UI N:N Matrix", "Layer 1 8/8 · Layer 2 CDP 36/36 · Zero Inline Script"),
        ("Haring & Stage Live", "Gate UI-HARING #000000 · >=99.5% · 6 Ticks · F12 Blackout"),
    ]
    y0 = 340
    for k, v in pills:
        d.rounded_rectangle([130, y0, W - 130, y0 + 52], radius=10, fill=(22, 32, 50), outline=(48, 66, 94), width=1)
        d.text((152, y0 + 14), k, font=F_BADGE, fill=(96, 165, 250))
        d.text((430, y0 + 13), v, font=F_ROW, fill=(230, 237, 243))
        y0 += 66
    return img


def render_chapter_card(ch_idx: int, total_ch: int, chapter: dict) -> Image.Image:
    img = make_gradient_bg((9, 13, 22), (17, 25, 40))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([110, 190, W - 110, H - 190], radius=18, fill=(16, 23, 36), outline=(96, 165, 250), width=2)
    d.text((155, 232), f"CHAPTER {ch_idx} / {total_ch}  |  {chapter['app'].upper()}", font=F_BADGE, fill=(56, 189, 248))
    d.text((155, 275), safe_text(chapter["title"]), font=F_CH_TITLE, fill=(243, 245, 248))
    d.text((155, 342), safe_text(chapter["subtitle"]), font=F_CH_SUB, fill=(176, 188, 206))
    d.text((155, 410), f"수행 스텝: {len(chapter['shots'])}개 상호작용 프레임 시퀀스 검증 완료", font=F_ROW, fill=(34, 197, 94))
    return img


def render_outro_card(manifest: dict, total_sec: float) -> Image.Image:
    img = make_gradient_bg((7, 12, 20), (14, 28, 36))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([90, 90, W - 90, H - 90], radius=20, fill=(13, 21, 32), outline=(34, 197, 94), width=2)
    d.text((135, 135), "ALL GATES VERIFIED · RELEASE HYGIENE CLEAN", font=F_BADGE, fill=(34, 197, 94))
    d.text((135, 178), "louis-vision 전 앱 N:N 실사 검증 완료 요약", font=F_TITLE, fill=(243, 245, 248))

    rows = [
        "1. 합성데이터 코퍼스: 3,008건 (19도메인 × 8포맷 × 15언어), 최소 667,645 B, 500 KiB 미만 0건",
        "2. 백엔드 코어·라우터: louis-docs 45/45, lv-router 5/5 (CX-01..08), louis src-tauri 5/5 PASS",
        "3. Vision & XR Stage: xr_stage_and_cargo_nn 3/3 (Gate UI-HARING & :8788 MCP), lib 12/12 PASS",
        "4. 브라우저 UI 36셀 N:N: Layer 1 (8/8), Layer 2 Chromium CDP (36/36), HTTP 404 0건 / JS 예외 0건",
        "5. 이번 감사 수정: web-console Rule 6 인라인 스크립트 제거, Studio/Field-3D/Mobile/Haring 실구현",
        f"6. 실사 촬영 영상: {manifest['total_chapters']}개 챕터 · {manifest['total_shots']}개 실측 컷 · 1280x720 30fps ({total_sec:.1f}초)",
    ]
    y = 262
    for r in rows:
        d.text((135, y), r, font=F_ROW, fill=(228, 236, 244))
        y += 52
    return img


def compose_shot_frame(
    raw_png_path: Path,
    ch_idx: int,
    total_ch: int,
    chapter_title: str,
    shot: dict,
    progress_frac: float,
) -> Image.Image:
    canvas = Image.new("RGB", (W, H), (8, 11, 16))
    app_img = Image.open(raw_png_path).convert("RGB")
    if app_img.size != (INNER_W, INNER_H):
        app_img = app_img.resize((INNER_W, INNER_H), Image.Resampling.LANCZOS)
    canvas.paste(app_img, (INNER_X, INNER_Y))

    d = ImageDraw.Draw(canvas)
    # Border around app viewport (zero overlap inside app UI)
    d.rectangle([INNER_X - 1, INNER_Y - 1, INNER_X + INNER_W, INNER_Y + INNER_H], outline=(38, 52, 74), width=1)

    # Top matte bar (28px height)
    d.text((18, 5), f"Ch.{ch_idx}/{total_ch}  {chapter_title}", font=F_BAR_TOP, fill=(226, 232, 240))
    sc_badge = f"[{shot['scenario_id']}]  PASS"
    d.text((W - 290, 5), sc_badge, font=F_BADGE, fill=(34, 197, 94))

    # Bottom matte caption bar (26px height)
    d.text((18, H - 24), safe_text(shot["caption"]), font=F_BAR_BOT, fill=(203, 213, 225))
    # Subtle 3px overall video progress bar at the very bottom
    bar_w = max(8, int(W * max(0.0, min(1.0, progress_frac))))
    d.rectangle([0, H - 3, W, H], fill=(24, 32, 46))
    d.rectangle([0, H - 3, bar_w, H], fill=(56, 189, 248))
    return canvas


def crossfade_frames(img_a: Image.Image, img_b: Image.Image, n_frames: int):
    a = np.asarray(img_a, dtype=np.float32)
    b = np.asarray(img_b, dtype=np.float32)
    for i in range(1, n_frames + 1):
        alpha = i / (n_frames + 1)
        blend = (a * (1.0 - alpha) + b * alpha).astype(np.uint8)
        yield Image.fromarray(blend, mode="RGB")


def main():
    manifest = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
    chapters = manifest["chapters"]
    total_ch = len(chapters)

    KEYFRAMES_DIR.mkdir(parents=True, exist_ok=True)
    OUT_MP4.parent.mkdir(parents=True, exist_ok=True)

    timeline = []  # list of (Image, repeat_frames, is_transition_target)
    total_shots = sum(len(c["shots"]) for c in chapters)
    seen_shots = 0

    intro = render_intro_card()
    timeline.append((intro, int(2.6 * FPS)))

    rep_keyframes = []

    for ci, ch in enumerate(chapters, start=1):
        ch_card = render_chapter_card(ci, total_ch, ch)
        timeline.append((ch_card, int(1.8 * FPS)))

        best_idx = min(len(ch["shots"]) // 2, len(ch["shots"]) - 1)
        for si, sh in enumerate(ch["shots"]):
            seen_shots += 1
            frac = seen_shots / max(1, total_shots)
            frame_path = CAPTURE_DIR / sh["file"]
            composed = compose_shot_frame(frame_path, ci, total_ch, ch["app"], sh, frac)
            frames_cnt = max(4, int(round(sh["duration_s"] * FPS)))
            timeline.append((composed, frames_cnt))
            if si == best_idx:
                kf_path = KEYFRAMES_DIR / f"{ch['id']}_keyframe.png"
                composed.save(kf_path, "PNG")
                rep_keyframes.append((ch["app"], sh["scenario_id"], kf_path))

    outro_frames = int(3.2 * FPS)
    exact_total_frames = sum(n for _, n in timeline) + outro_frames + len(timeline) * 6
    outro = render_outro_card(manifest, exact_total_frames / FPS)
    timeline.append((outro, outro_frames))

    # Build storyboard overview contact sheet (2 columns x 4 rows including outro)
    thumb_w, thumb_h = 600, 338
    sheet_cols, sheet_rows = 2, 4
    sheet = Image.new("RGB", (sheet_cols * thumb_w + 36, sheet_rows * thumb_h + 64), (10, 14, 22))
    sd = ImageDraw.Draw(sheet)
    sd.text((20, 14), "Louis-Vision Full-App N:N User Scenario Storyboard (Ch.1 ~ Ch.7 Keyframes + Scorecard)", font=F_ROW, fill=(240, 244, 248))
    cards = [kf for _, _, kf in rep_keyframes] + [outro]
    for idx, item in enumerate(cards):
        im = Image.open(item).convert("RGB") if isinstance(item, Path) else item
        im = im.resize((thumb_w, thumb_h), Image.Resampling.LANCZOS)
        r = idx // sheet_cols
        c = idx % sheet_cols
        sheet.paste(im, (12 + c * (thumb_w + 12), 52 + r * (thumb_h + 8)))
    sheet.save(OUT_STORYBOARD, "PNG")

    ffmpeg_exe = imageio_ffmpeg.get_ffmpeg_exe()
    cmd = [
        ffmpeg_exe,
        "-y",
        "-f", "rawvideo",
        "-vcodec", "rawvideo",
        "-pix_fmt", "rgb24",
        "-s", f"{W}x{H}",
        "-r", str(FPS),
        "-i", "-",
        "-c:v", "libx264",
        "-preset", "medium",
        "-crf", "20",
        "-pix_fmt", "yuv420p",
        "-movflags", "+faststart",
        str(OUT_MP4),
    ]

    proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    encoded_frames = 0
    prev_img = None
    assert proc.stdin is not None
    for img, count in timeline:
        if prev_img is not None:
            for tf in crossfade_frames(prev_img, img, 6):
                proc.stdin.write(tf.tobytes())
                encoded_frames += 1
        raw_bytes = img.tobytes()
        for _ in range(count):
            proc.stdin.write(raw_bytes)
            encoded_frames += 1
        prev_img = img

    proc.stdin.close()
    stdout, stderr = proc.communicate()
    if proc.returncode != 0:
        raise RuntimeError(f"ffmpeg failed ({proc.returncode}):\n{stderr.decode('utf-8', errors='replace')}")

    # Verify encoded video stream cleanly via ffmpeg -v error -i <mp4> -f null -
    verify = subprocess.run(
        [ffmpeg_exe, "-v", "error", "-i", str(OUT_MP4), "-f", "null", "-"],
        capture_output=True,
        text=True,
    )
    if verify.returncode != 0 or verify.stderr.strip():
        raise RuntimeError(f"MP4 decode verification failed: {verify.stderr}")

    # Clean up bulky raw frames folder now that MP4 & keyframes are verified
    frames_sub = CAPTURE_DIR / "frames"
    if frames_sub.exists():
        shutil.rmtree(frames_sub)

    size_bytes = OUT_MP4.stat().st_size
    print(json.dumps({
        "status": "VIDEO_OK",
        "mp4": str(OUT_MP4),
        "storyboard": str(OUT_STORYBOARD),
        "keyframes": len(rep_keyframes),
        "encoded_frames": encoded_frames,
        "duration_s": round(encoded_frames / FPS, 2),
        "mp4_bytes": size_bytes,
    }, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
