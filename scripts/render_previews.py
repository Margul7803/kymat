import math
import numpy as np
from PIL import Image, ImageDraw, ImageFont

W, H = 1440, 810
bold_font_path = "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Bold.ttf"
reg_font_path = "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Regular.ttf"

font_title = ImageFont.truetype(bold_font_path, 18)
font_artist = ImageFont.truetype(reg_font_path, 13)
font_ts = ImageFont.truetype(reg_font_path, 11)

BAYER_4X4 = [
    [-0.46875, 0.03125, -0.34375, 0.15625],
    [0.28125, -0.21875, 0.40625, -0.09375],
    [-0.28125, 0.21875, -0.40625, 0.09375],
    [0.46875, -0.03125, 0.34375, -0.15625],
]

def create_ambient_background(split_t=0.0):
    bg = np.zeros((H, W, 3), dtype=np.float32)
    p1_x = 0.46 + 0.22 * split_t
    p1_y = 0.33
    p2_x = 0.38
    p2_y = 0.68

    c_base = np.array([5.0, 5.0, 8.0])
    c_glow1 = np.array([42.0, 16.0, 36.0])
    c_glow2 = np.array([12.0, 24.0, 48.0])

    y_coords, x_coords = np.mgrid[0:H, 0:W]
    u = x_coords / float(W - 1)
    v = y_coords / float(H - 1)

    dx1 = (u - p1_x) * 1.35
    dy1 = (v - p1_y) * 1.55
    w1 = np.exp(-3.2 * (dx1 * dx1 + dy1 * dy1)) * 0.85

    dx2 = (u - p2_x) * 1.45
    dy2 = (v - p2_y) * 1.65
    w2 = np.exp(-3.6 * (dx2 * dx2 + dy2 * dy2)) * 0.72

    edge_dx = (u - 0.5) * 2.0
    edge_dy = (v - 0.5) * 2.0
    edge_dist = np.minimum(edge_dx * edge_dx + edge_dy * edge_dy, 1.4)
    vignette = np.clip(1.0 - edge_dist * 0.38, 0.35, 1.0)

    for i in range(3):
        comp = c_base[i] + (c_glow1[i] - c_base[i]) * (w1 * vignette) + (c_glow2[i] - c_base[i]) * (w2 * vignette)
        bg[:, :, i] = comp

    dither = np.tile(BAYER_4X4, (H // 4 + 1, W // 4 + 1))[:H, :W] * 1.4
    bg = np.clip(bg + dither[:, :, None], 0, 255).astype(np.uint8)
    return Image.fromarray(bg, "RGB").convert("RGBA")

def draw_soundwave(im):
    draw = ImageDraw.Draw(im)
    wave_base_y = H - 1
    max_h = 195.0

    # 1. Semi-transparent gradient fill below the primary curve
    fill_overlay = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    fill_draw = ImageDraw.Draw(fill_overlay)

    xs = list(range(0, W, 2))
    points_h1 = []
    points_h2 = []

    for x in xs:
        u = x / float(W)
        m1 = math.sin(u * 13.5 - 2.1) * 0.44 + math.sin(u * 25.0 + 1.2) * 0.32 + math.cos(u * 7.5) * 0.24
        val1 = (m1 * 0.5 + 0.5) ** 1.35
        h1 = val1 * max_h + 16.0
        y1 = wave_base_y - int(h1)
        points_h1.append((x, y1))

        m2 = math.sin(u * 17.5 + 0.8) * 0.5 + 0.5
        val2 = (m2 ** 1.25)
        h2 = val2 * max_h * 0.78 + 12.0
        y2 = wave_base_y - int(h2)
        points_h2.append((x, y2))

    # Fill polygon under curve 1
    poly = [(0, wave_base_y)] + points_h1 + [(W, wave_base_y)]
    fill_draw.polygon(poly, fill=(28, 52, 115, 85))

    # Secondary softer fill under curve 2
    poly2 = [(0, wave_base_y)] + points_h2 + [(W, wave_base_y)]
    fill_draw.polygon(poly2, fill=(18, 36, 85, 60))

    # Vertical gradient mask over fill
    im.paste(Image.alpha_composite(Image.new("RGBA", (W, H), (0, 0, 0, 0)), fill_overlay), (0, 0), fill_overlay)

    # 2. Draw crisp double-braille crest dots
    for (x, y1), (_, y2) in zip(points_h1, points_h2):
        if x % 4 != 0:
            continue
        # Primary crest
        draw.rectangle([(x, y1 - 1), (x + 1, y1 + 1)], fill=(185, 215, 255, 245))
        draw.point((x, y1 - 2), fill=(230, 242, 255, 255))
        draw.point((x + 1, y1 + 2), fill=(135, 175, 255, 200))

        # Secondary crest
        draw.rectangle([(x, y2), (x + 1, y2 + 1)], fill=(95, 135, 225, 180))

def draw_time_bar(draw, center_x, bar_y, bar_w=340):
    bar_x = center_x - bar_w // 2
    left_ts = "1:42"
    right_ts = "7:36"

    # Dark backdrop pill so text and bar never clash with wave lines behind them
    bbox_l = draw.textbbox((0, 0), left_ts, font=font_ts)
    lw = bbox_l[2] - bbox_l[0]
    pill_x0 = bar_x - lw - 18
    pill_x1 = bar_x + bar_w + lw + 18
    draw.rounded_rectangle([(pill_x0, bar_y - 10), (pill_x1, bar_y + 11)], radius=7, fill=(7, 9, 14, 225))

    # Base track and active progress
    draw.line([(bar_x, bar_y), (bar_x + bar_w, bar_y)], fill=(44, 52, 68, 255), width=3)
    progress_w = int(bar_w * 0.22)
    draw.line([(bar_x, bar_y), (bar_x + progress_w, bar_y)], fill=(125, 155, 250, 255), width=3)
    draw.ellipse([(bar_x + progress_w - 4, bar_y - 4), (bar_x + progress_w + 4, bar_y + 4)], fill=(255, 255, 255, 255))

    draw.text((bar_x - lw - 12, bar_y - 7), left_ts, font=font_ts, fill=(160, 168, 182, 255))
    draw.text((bar_x + bar_w + 12, bar_y - 7), right_ts, font=font_ts, fill=(160, 168, 182, 255))

# 1. Lyrics View
def render_lyrics_view():
    im = create_ambient_background(split_t=1.0)
    draw = ImageDraw.Draw(im)

    cover_src = Image.open("/home/mario/.gemini/antigravity-cli/brain/d424bcd1-c5ac-4130-9797-5d931c58523d/scratch/cuco_cover.png").convert("RGBA")
    cw, ch = 396, 396
    cx = 112
    cy = 138
    cover_resized = cover_src.resize((cw, ch), Image.Resampling.LANCZOS)
    im.paste(cover_resized, (cx, cy), cover_resized)

    center_x = cx + cw // 2
    title_text = "Lover Is a Day "
    heart_text = "\uf004"
    artist_text = "Cuco"

    bbox_t = draw.textbbox((0, 0), title_text, font=font_title)
    bbox_h = draw.textbbox((0, 0), heart_text, font=font_title)
    tw = bbox_t[2] - bbox_t[0]
    hw = bbox_h[2] - bbox_h[0]
    total_w = tw + hw

    tx = center_x - total_w // 2
    ty = cy + ch + 22
    draw.text((tx, ty), title_text, font=font_title, fill=(255, 255, 255, 255))
    draw.text((tx + tw, ty), heart_text, font=font_title, fill=(235, 105, 145, 255))

    bbox_a = draw.textbbox((0, 0), artist_text, font=font_artist)
    aw = bbox_a[2] - bbox_a[0]
    ax = center_x - aw // 2
    ay = ty + 26
    draw.text((ax, ay), artist_text, font=font_artist, fill=(160, 168, 182, 255))

    # Time bar beneath artist
    draw_time_bar(draw, center_x, ay + 32, bar_w=260)

    # Lyrics column on right
    lyrics_img = Image.open("assets/lyrics-view.png").convert("RGBA")
    right_lyrics = lyrics_img.crop((640, 0, W, H))
    im.paste(right_lyrics, (640, 0), right_lyrics)

    im.convert("RGB").save("assets/lyrics-view.png", quality=95)
    print("Saved assets/lyrics-view.png")

# 2. Soundwave View
def render_wave_view():
    im = create_ambient_background(split_t=0.0)
    draw = ImageDraw.Draw(im)

    # Soundwave across lower section
    draw_soundwave(im)

    # Centered 1:1 square cover
    cover_src = Image.open("/home/mario/.gemini/antigravity-cli/brain/d424bcd1-c5ac-4130-9797-5d931c58523d/scratch/cuco_cover.png").convert("RGBA")
    cw, ch = 336, 336
    cx = (W - cw) // 2
    cy = 82
    cover_resized = cover_src.resize((cw, ch), Image.Resampling.LANCZOS)
    im.paste(cover_resized, (cx, cy), cover_resized)

    center_x = W // 2
    title_text = "Lover Is a Day "
    heart_text = "\uf004"
    artist_text = "Cuco"

    bbox_t = draw.textbbox((0, 0), title_text, font=font_title)
    bbox_h = draw.textbbox((0, 0), heart_text, font=font_title)
    tw = bbox_t[2] - bbox_t[0]
    hw = bbox_h[2] - bbox_h[0]
    total_w = tw + hw

    tx = center_x - total_w // 2
    ty = cy + ch + 20
    draw.text((tx, ty), title_text, font=font_title, fill=(255, 255, 255, 255))
    draw.text((tx + tw, ty), heart_text, font=font_title, fill=(235, 105, 145, 255))

    bbox_a = draw.textbbox((0, 0), artist_text, font=font_artist)
    aw = bbox_a[2] - bbox_a[0]
    ax = center_x - aw // 2
    ay = ty + 26
    draw.text((ax, ay), artist_text, font=font_artist, fill=(160, 168, 182, 255))

    # Time bar cleanly floating at bottom
    draw_time_bar(draw, center_x, H - 42, bar_w=340)

    im.convert("RGB").save("assets/wave-view.png", quality=95)
    print("Saved assets/wave-view.png")

# 3. Dancing Rat Easter Egg View
def render_rat_view():
    im = create_ambient_background(split_t=0.0)
    draw = ImageDraw.Draw(im)

    # Soundwave at bottom
    draw_soundwave(im)

    # Clean transparent rat frame directly from sprite sheet
    sheet = Image.open("assets/rat_spritesheet.png").convert("RGBA")
    frame10 = sheet.crop((0, 10 * 120, 80, 11 * 120))

    rw, rh = 340, 510
    rat_resized = frame10.resize((rw, rh), Image.Resampling.LANCZOS)
    rx = (W - rw) // 2
    ry = (H - rh) // 2 - 25

    im.paste(rat_resized, (rx, ry), rat_resized)

    # Top indicator
    label = "kymat · easter egg active ('r' to toggle)"
    bbox = draw.textbbox((0, 0), label, font=font_artist)
    lw = bbox[2] - bbox[0]
    draw.text(((W - lw) // 2, 28), label, font=font_artist, fill=(235, 105, 145, 240))

    # Time bar at bottom
    draw_time_bar(draw, W // 2, H - 42, bar_w=340)

    im.convert("RGB").save("assets/easter-egg-view.png", quality=95)
    print("Saved assets/easter-egg-view.png")

render_lyrics_view()
render_wave_view()
render_rat_view()
