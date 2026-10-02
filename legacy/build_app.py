import os
import subprocess
import sys
from PIL import Image, ImageDraw

BASE_DIR = os.path.dirname(os.path.abspath(__file__))
OUTPUT_DIR = os.path.join(BASE_DIR, "dist")

def create_play_icon(size=64):
    """Draws a premium electric cyan play button inside a circle with transparent background."""
    image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    
    stroke_width = max(2, int(size * 0.08))
    padding = stroke_width + 2
    circle_box = [padding, padding, size - padding, size - padding]
    
    # Premium vibrant cyan color
    color = (0, 210, 255, 255)
    
    # Draw circle outline
    draw.ellipse(circle_box, outline=color, width=stroke_width)
    
    # Draw play triangle
    center_x = size / 2
    center_y = size / 2
    r = size * 0.20
    
    # Visual center offset for the triangle
    offset_x = size * 0.05
    
    x1 = center_x + r + offset_x
    y1 = center_y
    
    x2 = center_x - (r * 0.7) + offset_x
    y2 = center_y - (r * 0.8)
    
    x3 = center_x - (r * 0.7) + offset_x
    y3 = center_y + (r * 0.8)
    
    draw.polygon([(x1, y1), (x2, y2), (x3, y3)], fill=color)
    return image

def build_icon():
    print("Generating multi-resolution icon.ico...")
    sizes = [16, 24, 32, 48, 64, 128, 256]
    images = []
    
    # Generate images for each size
    for size in sizes:
        images.append(create_play_icon(size))
        
    # Save as a single .ico file
    images[0].save(
        os.path.join(BASE_DIR, "icon.ico"),
        format="ICO",
        sizes=[(size, size) for size in sizes],
        append_images=images[1:]
    )
    print("icon.ico generated successfully.")

def run_pyinstaller():
    print("Compiling tray_app.py to executable using PyInstaller...")
    # --noconsole prevents the command prompt window from appearing when the app runs
    # --onefile packages everything into a single EXE
    # --icon applies the custom .ico file to the compiled executable
    # --name names the output executable
    os.makedirs(OUTPUT_DIR, exist_ok=True)
    cmd = [
        sys.executable,
        "-m",
        "PyInstaller",
        "--noconfirm",
        "--clean",
        "--noconsole",
        "--onefile",
        f"--icon={os.path.join(BASE_DIR, 'icon.ico')}",
        "--name=PoymaiDeploy",
        f"--distpath={OUTPUT_DIR}",
        f"--workpath={os.path.join(BASE_DIR, 'build')}",
        f"--specpath={BASE_DIR}",
        os.path.join(BASE_DIR, "tray_app.py"),
    ]
    
    result = subprocess.run(
        cmd,
        cwd=BASE_DIR,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if result.returncode == 0:
        executable_path = os.path.join(OUTPUT_DIR, "PoymaiDeploy.exe")
        if not os.path.isfile(executable_path):
            raise FileNotFoundError(f"PyInstaller did not create '{executable_path}'.")
        print("Compilation successful!")
        print(f"The executable is located at '{executable_path}'.")
    else:
        print("Compilation failed.")
        print("Error details:")
        print(result.stdout)
        print(result.stderr)
        raise SystemExit(result.returncode)

if __name__ == "__main__":
    os.chdir(BASE_DIR)
    build_icon()
    run_pyinstaller()
