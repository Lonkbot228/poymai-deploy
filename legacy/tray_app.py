import sys
import os
import io
import re

# Redirect stdout and stderr to a log file to prevent AttributeError: 'NoneType' object has no attribute 'write'
# in PyInstaller windowless (--noconsole) mode, and to capture any error output.
try:
    os.makedirs(r"C:\code\poymai", exist_ok=True)
    # Open log file in append mode. Line buffering ensures immediate writes.
    log_file = open(r"C:\code\poymai\app_log.txt", "a", encoding="utf-8", buffering=1)
    if sys.stdout is None:
        sys.stdout = log_file
    if sys.stderr is None:
        sys.stderr = log_file
except Exception:
    # Fallback if log file cannot be created
    if sys.stdout is None:
        sys.stdout = io.StringIO()
    if sys.stderr is None:
        sys.stderr = io.StringIO()

import subprocess
import pystray
from PIL import Image, ImageDraw
import tkinter as tk
from tkinter import scrolledtext
import threading
import json
import time

# Define paths
SCRIPT_FOLDER = r"C:\code\poymai"
SCRIPT_PATH = os.path.join(SCRIPT_FOLDER, "deploy.ps1")
TELEGRAM_SCRIPT_FOLDER = r"C:\code\poymaitelegram"
TELEGRAM_SCRIPT_PATH = os.path.join(TELEGRAM_SCRIPT_FOLDER, "deploy.ps1")
STARTUP_SHORTCUT_NAME = "PoymaiDeploy.lnk"
CREATE_NO_WINDOW = 0x08000000 if os.name == "nt" else 0
ANSI_ESCAPE_RE = re.compile(r"\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])")
APP_VERSION = "2.0.0"

def clean_process_output(value):
    """Normalizes PowerShell/native output for the popup and UTF-8 log."""
    return ANSI_ESCAPE_RE.sub("", value or "").replace("\x00", "").strip()

def get_powershell_command(script_path):
    """Builds a deterministic UTF-8 PowerShell invocation for a deploy script."""
    system_root = os.environ.get("SystemRoot", r"C:\Windows")
    powershell_path = os.path.join(
        system_root,
        "System32",
        "WindowsPowerShell",
        "v1.0",
        "powershell.exe",
    )
    escaped_script_path = script_path.replace("'", "''")
    command = (
        "[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false); "
        "$ProgressPreference = 'SilentlyContinue'; "
        f"& '{escaped_script_path}'; "
        "if ($null -ne $LASTEXITCODE) { exit $LASTEXITCODE }"
    )
    return [
        powershell_path,
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        command,
    ]

def get_startup_shortcut_path():
    startup_folder = os.path.join(os.environ['APPDATA'], r'Microsoft\Windows\Start Menu\Programs\Startup')
    return os.path.join(startup_folder, STARTUP_SHORTCUT_NAME)

def check_or_create_script():
    """Ensures the script folder and deploy.ps1 file exist, creating a template if missing."""
    try:
        if not os.path.exists(SCRIPT_FOLDER):
            os.makedirs(SCRIPT_FOLDER, exist_ok=True)
        
        if not os.path.exists(SCRIPT_PATH):
            with open(SCRIPT_PATH, "w", encoding="utf-8") as f:
                f.write("# Poymai Deploy Script\n")
                f.write("Write-Host '===================================================' -ForegroundColor Cyan\n")
                f.write("Write-Host '            Poymai Deploy Active' -ForegroundColor Cyan\n")
                f.write("Write-Host '===================================================' -ForegroundColor Cyan\n")
                f.write("Write-Host 'Starting file deployment to your server...'\n")
                f.write("Start-Sleep -Seconds 2\n")
                f.write("Write-Host 'Files uploaded successfully!' -ForegroundColor Green\n")
    except Exception as e:
        print(f"Error creating script template: {e}")

def create_shortcut(shortcut_path, target_path, working_dir, arguments=""):
    """Creates a Windows shortcut (.lnk) using PowerShell."""
    powershell_cmd = f"""
    $WshShell = New-Object -ComObject WScript.Shell
    $Shortcut = $WshShell.CreateShortcut("{shortcut_path}")
    $Shortcut.TargetPath = "{target_path}"
    $Shortcut.WorkingDirectory = "{working_dir}"
    if ("{arguments}" -ne "") {{
        $Shortcut.Arguments = "{arguments}"
    }}
    $Shortcut.Save()
    """
    subprocess.run(["powershell", "-Command", powershell_cmd], capture_output=True, text=True, check=True)

def open_folder(icon, item):
    """Opens the folder containing the deploy script in File Explorer."""
    if not os.path.exists(SCRIPT_FOLDER):
        os.makedirs(SCRIPT_FOLDER, exist_ok=True)
    os.startfile(SCRIPT_FOLDER)

def is_startup_enabled(item):
    """Checks if the app shortcut exists in the Windows Startup folder."""
    return os.path.exists(get_startup_shortcut_path())

def toggle_startup(icon, item):
    """Toggles the app starting with Windows."""
    shortcut_path = get_startup_shortcut_path()
    if os.path.exists(shortcut_path):
        try:
            os.remove(shortcut_path)
        except Exception as e:
            pass
    else:
        try:
            if getattr(sys, 'frozen', False):
                target_path = sys.executable
                working_dir = os.path.dirname(sys.executable)
                create_shortcut(shortcut_path, target_path, working_dir)
            else:
                target_path = sys.executable
                working_dir = os.path.dirname(os.path.abspath(__file__))
                script_path = os.path.abspath(__file__)
                create_shortcut(shortcut_path, target_path, working_dir, f'"{script_path}"')
        except Exception as e:
            pass

def create_play_icon(size=64):
    """Draws a premium electric cyan play button inside a circle with transparent background."""
    image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    
    stroke_width = max(2, int(size * 0.08))
    padding = stroke_width + 2
    circle_box = [padding, padding, size - padding, size - padding]
    
    color = (0, 210, 255, 255) # Electric cyan
    
    # Draw circle outline
    draw.ellipse(circle_box, outline=color, width=stroke_width)
    
    # Draw play triangle
    center_x = size / 2
    center_y = size / 2
    r = size * 0.20
    offset_x = size * 0.05
    
    x1 = center_x + r + offset_x
    y1 = center_y
    x2 = center_x - (r * 0.7) + offset_x
    y2 = center_y - (r * 0.8)
    x3 = center_x - (r * 0.7) + offset_x
    y3 = center_y + (r * 0.8)
    
    draw.polygon([(x1, y1), (x2, y2), (x3, y3)], fill=color)
    return image

def create_progress_icon(progress, size=64):
    """Draws a transparent square that fills up from bottom to top in electric cyan."""
    image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    
    stroke_width = max(2, int(size * 0.08))
    padding = stroke_width + 4
    square_box = [padding, padding, size - padding, size - padding]
    
    color = (0, 210, 255, 255) # Electric cyan
    
    # Outline square
    draw.rectangle(square_box, outline=color, width=stroke_width)
    
    # Calculate fill height
    inner_padding = padding + stroke_width
    x0 = inner_padding
    y0 = inner_padding
    x1 = size - inner_padding
    y1 = size - inner_padding
    
    total_height = y1 - y0
    fill_height = int(total_height * (progress / 100.0))
    
    if fill_height > 0:
        # Fill from bottom to top
        fill_box = [x0 + 1, y1 - fill_height, x1 - 1, y1]
        draw.rectangle(fill_box, fill=color)
        
    return image

def create_error_icon(size=64):
    """Draws a warning exclamation mark inside a red square outline."""
    image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    
    stroke_width = max(2, int(size * 0.08))
    padding = stroke_width + 4
    square_box = [padding, padding, size - padding, size - padding]
    
    red_color = (248, 113, 113, 255) # Red-400
    
    # Draw red square outline
    draw.rectangle(square_box, outline=red_color, width=stroke_width)
    
    # Draw an exclamation mark (!) in the center
    center_x = size / 2
    
    # Top line of the exclamation mark
    draw.line([(center_x, size * 0.3), (center_x, size * 0.6)], fill=red_color, width=max(2, int(size * 0.06)))
    # Bottom dot
    dot_radius = max(1.5, size * 0.04)
    draw.ellipse([center_x - dot_radius, size * 0.7 - dot_radius, center_x + dot_radius, size * 0.7 + dot_radius], fill=red_color)
    
    return image

# Thread-safe Deployment State
class DeployState:
    def __init__(self):
        self.lock = threading.Lock()
        self.is_deploying = False
        self.progress = 0
        self.stage = "Ожидание..."
        self.sub_status = ""
        self.status = "idle"  # idle, running, success, error
        self.error_text = ""
        self.deployment_title = "POYMAI DEPLOYER"
        self.pystray_icon = None

    def update(self, **kwargs):
        with self.lock:
            for k, v in kwargs.items():
                setattr(self, k, v)
            
            # Immediately update the pystray icon if progress or status changes
            if self.pystray_icon:
                if self.status == "running":
                    self.pystray_icon.icon = create_progress_icon(self.progress)
                elif self.status == "error":
                    self.pystray_icon.icon = create_error_icon()
                elif self.status == "success" or self.status == "idle":
                    self.pystray_icon.icon = create_play_icon()

state = DeployState()

def get_target_monitor_coords():
    """Queries Windows for screen coordinates using native Win32 APIs, mapping friendly names."""
    import ctypes
    from ctypes import wintypes

    # Win32 structures
    class DISPLAY_DEVICE(ctypes.Structure):
        _fields_ = [
            ('cb', wintypes.DWORD),
            ('DeviceName', ctypes.c_wchar * 32),
            ('DeviceString', ctypes.c_wchar * 128),
            ('StateFlags', wintypes.DWORD),
            ('DeviceID', ctypes.c_wchar * 128),
            ('DeviceKey', ctypes.c_wchar * 128)
        ]

    class MONITORINFOEXW(ctypes.Structure):
        _fields_ = [
            ('cbSize', wintypes.DWORD),
            ('rcMonitor', wintypes.RECT),
            ('rcWork', wintypes.RECT),
            ('dwFlags', wintypes.DWORD),
            ('szDevice', ctypes.c_wchar * 32)
        ]

    try:
        user32 = ctypes.windll.user32
        
        # 1. Map Adapter Device Name (like "\\\\.\\DISPLAY1") to Friendly Monitor Names
        device_to_friendly = {}
        adapter_index = 0
        while True:
            adapter = DISPLAY_DEVICE()
            adapter.cb = ctypes.sizeof(DISPLAY_DEVICE)
            res = user32.EnumDisplayDevicesW(None, adapter_index, ctypes.byref(adapter), 0)
            if not res:
                break
                
            adapter_name = adapter.DeviceName
            
            monitor_index = 0
            while True:
                monitor = DISPLAY_DEVICE()
                monitor.cb = ctypes.sizeof(DISPLAY_DEVICE)
                res2 = user32.EnumDisplayDevicesW(adapter_name, monitor_index, ctypes.byref(monitor), 0)
                if not res2:
                    break
                
                if adapter_name not in device_to_friendly:
                    device_to_friendly[adapter_name] = []
                device_to_friendly[adapter_name].append(monitor.DeviceString)
                monitor_index += 1
                
            adapter_index += 1

        # 2. Get Coordinate Info for all active monitors
        screens = []
        MONITORENUMPROC = ctypes.WINFUNCTYPE(
            wintypes.BOOL,
            wintypes.HMONITOR,
            wintypes.HDC,
            ctypes.POINTER(wintypes.RECT),
            wintypes.LPARAM
        )
        
        def callback(hMonitor, hdcMonitor, lprcMonitor, dwData):
            info = MONITORINFOEXW()
            info.cbSize = ctypes.sizeof(MONITORINFOEXW)
            if user32.GetMonitorInfoW(hMonitor, ctypes.byref(info)):
                dev_name = info.szDevice
                friendly_names = device_to_friendly.get(dev_name, ["Generic PnP Monitor"])
                friendly_name = friendly_names[0] if friendly_names else "Generic PnP Monitor"
                
                screens.append({
                    "DeviceName": dev_name,
                    "FriendlyName": friendly_name,
                    "Primary": (info.dwFlags & 1) != 0, # MONITORINFOF_PRIMARY = 1
                    "WorkingX": info.rcWork.left,
                    "WorkingY": info.rcWork.top,
                    "WorkingWidth": info.rcWork.right - info.rcWork.left,
                    "WorkingHeight": info.rcWork.bottom - info.rcWork.top
                })
            return True

        user32.EnumDisplayMonitors(None, None, MONITORENUMPROC(callback), 0)
        
        # 3. Find target monitor: search for "VA2719"
        for scr in screens:
            if "VA2719" in scr["FriendlyName"]:
                return scr
                
        # 4. Fallback: find primary monitor
        for scr in screens:
            if scr["Primary"]:
                return scr
                
        # 5. Final fallback
        if screens:
            return screens[0]
            
    except Exception as e:
        print(f"Error in win32 monitor detection: {e}")
        
    return {"WorkingX": 0, "WorkingY": 0, "WorkingWidth": 1920, "WorkingHeight": 1080, "Primary": True}

MAIN_STAGE_MAP = {
    "Git backup": (10, "Резервное копирование Git..."),
    "Frontend production build": (30, "Сборка Next.js (это займет ~10-15 сек)..."),
    "Preparing archive": (60, "Сжатие проекта в архив..."),
    "Preparing remote directory": (70, "Подготовка папки на сервере..."),
    "Uploading archive": (80, "Загрузка архива на сервер..."),
    "Extracting files on server": (85, "Распаковка на сервере..."),
    "Rebuilding Docker services": (90, "Перезапуск контейнеров Docker..."),
    "Deploy completed": (100, "Деплой успешно завершен!"),
}

TELEGRAM_STAGE_MAP = {
    "Telegram preflight": (8, "Проверка Telegram-шлюза..."),
    "Preparing Telegram archive": (20, "Подготовка архива шлюза..."),
    "Preparing Telegram server": (35, "Подготовка сервера Telegram..."),
    "Uploading Telegram archive": (48, "Загрузка шлюза на сервер..."),
    "Validating Telegram release": (58, "Проверка кода на сервере..."),
    "Backing up Telegram release": (68, "Резервное копирование шлюза..."),
    "Installing Telegram release": (76, "Установка новой версии..."),
    "Rebuilding Telegram bridge": (86, "Пересборка Telegram-контейнера..."),
    "Checking Telegram health": (94, "Проверка запуска и соединений..."),
    "Telegram deploy completed": (100, "Telegram-шлюз успешно обновлен!"),
}

DEPLOY_PROFILES = {
    "main": {
        "title": "POYMAI DEPLOYER",
        "success_text": "Деплой успешно завершен!",
        "folder": SCRIPT_FOLDER,
        "script": SCRIPT_PATH,
        "log_name": "MAIN",
        "stage_map": MAIN_STAGE_MAP,
    },
    "telegram": {
        "title": "POYMAI TELEGRAM DEPLOY",
        "success_text": "Telegram-шлюз успешно обновлен!",
        "folder": TELEGRAM_SCRIPT_FOLDER,
        "script": TELEGRAM_SCRIPT_PATH,
        "log_name": "TELEGRAM",
        "stage_map": TELEGRAM_STAGE_MAP,
    },
}


def run_powershell_deploy_thread(profile):
    """Background thread to run the PowerShell script and parse output."""
    script_path = profile["script"]
    script_folder = profile["folder"]
    stage_map = profile["stage_map"]
    try:
        # Write launch information to app_log.txt
        with open(r"C:\code\poymai\app_log.txt", "a", encoding="utf-8") as f:
            f.write(
                f"\n--- {profile['log_name']} DEPLOY STARTED AT "
                f"{time.strftime('%Y-%m-%d %H:%M:%S')} ---\n"
            )
        
        if not os.path.isfile(script_path):
            raise FileNotFoundError(f"Deploy script was not found: {script_path}")

        # PowerShell is forced to UTF-8 and runs without user profiles so local
        # profile settings cannot turn harmless native warnings into failures.
        process = subprocess.Popen(
            get_powershell_command(script_path),
            cwd=script_folder,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            stdin=subprocess.DEVNULL,
            text=True,
            encoding="utf-8",
            errors="replace",
            bufsize=1,
            creationflags=CREATE_NO_WINDOW,
        )
        
        current_progress = 5
        current_stage = "Подключение..."
        state.update(progress=current_progress, stage=current_stage, sub_status="Запуск скрипта...")
        
        # Keep a running buffer of the last 100 lines for error tracebacks
        output_buffer = []
        
        # Thread to read stdout line by line
        for line in process.stdout:
            trimmed = clean_process_output(line)
            if not trimmed:
                continue
            
            # Save to buffer
            output_buffer.append(trimmed)
            if len(output_buffer) > 100:
                output_buffer.pop(0)
            
            # Log stdout to app_log.txt
            with open(r"C:\code\poymai\app_log.txt", "a", encoding="utf-8") as f:
                f.write(trimmed + "\n")
            
            # Check for stage header matching the Step function output: ==> Message
            if trimmed.startswith("==>"):
                stage_name = trimmed.replace("==>", "").strip()
                if stage_name in stage_map:
                    prog, text = stage_map[stage_name]
                    current_progress = prog
                    current_stage = text
                    state.update(progress=current_progress, stage=current_stage, sub_status="")
            else:
                # Update sub status with standard script outputs (e.g. Next.js pages)
                # Keep sub status short so it fits nicely
                if trimmed.lower().startswith(("npm warn", "npm notice")):
                    continue
                short_status = trimmed
                if len(short_status) > 55:
                    short_status = short_status[:52] + "..."
                state.update(sub_status=short_status)
        
        process.wait()
        
        if process.returncode == 0:
            state.update(
                progress=100,
                stage=profile["success_text"],
                sub_status="Все проверки выполнены.",
                status="success",
            )
        else:
            # Join buffer lines to create error traceback
            error_details = "\n".join(output_buffer)
            if not error_details:
                error_details = f"Процесс завершился с кодом ошибки {process.returncode}."
            
            # Log error details to app_log.txt
            with open(r"C:\code\poymai\app_log.txt", "a", encoding="utf-8") as f:
                f.write(f"\nERROR DETECTED:\n{error_details}\n")
            
            state.update(status="error", stage="Ошибка при деплое!", sub_status="Деплой аварийно остановлен.", error_text=error_details)
            
    except Exception as e:
        error_msg = f"Исключение при выполнении деплоя: {e}"
        print(error_msg)
        state.update(status="error", stage="Ошибка запуска!", sub_status="Не удалось выполнить скрипт.", error_text=error_msg)

def trigger_deploy_profile(profile_name):
    """Triggers one deploy profile while preventing concurrent deployments."""
    profile = DEPLOY_PROFILES[profile_name]
    # Prevent concurrent deployments
    with state.lock:
        if state.is_deploying:
            return
        state.is_deploying = True
        state.progress = 0
        state.stage = "Инициализация..."
        state.deployment_title = profile["title"]
    
    if profile_name == "main":
        check_or_create_script()
    
    # Reset state values
    state.update(status="running", progress=0, stage="Подготовка...", sub_status="Определение мониторов...", error_text="")
    
    # Start background execution thread
    t = threading.Thread(target=run_powershell_deploy_thread, args=(profile,))
    t.daemon = True
    t.start()


def trigger_deploy(icon=None, item=None):
    """Preserves the existing default PoymAI deployment action."""
    trigger_deploy_profile("main")


def trigger_telegram_deploy(icon=None, item=None):
    """Starts deployment of the personal Telegram bridge."""
    trigger_deploy_profile("telegram")

# UI Class for Floating Window
class ProgressWindow:
    def __init__(self, root):
        self.root = root
        self.root.overrideredirect(True)
        self.root.attributes("-topmost", True)
        self.root.configure(bg="#18181b")  # Dark zinc-900 background
        
        # Outer Border Frame (Cyan Glow effect)
        self.border_frame = tk.Frame(self.root, bg="#00d2ff", bd=0)
        self.border_frame.pack(fill="both", expand=True)
        
        # Inner Content Frame
        self.content_frame = tk.Frame(self.border_frame, bg="#18181b", bd=0)
        self.content_frame.pack(fill="both", expand=True, padx=1, pady=1)
        
        # Top Header (Title + Percentage)
        self.header_frame = tk.Frame(self.content_frame, bg="#18181b")
        self.header_frame.pack(fill="x", padx=12, pady=(10, 2))
        
        self.title_label = tk.Label(self.header_frame, text="POYMAI DEPLOYER", font=("Segoe UI", 8, "bold"), fg="#71717a", bg="#18181b")
        self.title_label.pack(side="left")
        
        self.percent_label = tk.Label(self.header_frame, text="0%", font=("Segoe UI", 8, "bold"), fg="#a1a1aa", bg="#18181b")
        self.percent_label.pack(side="right")
        
        # Stage Label (Large white bold text)
        self.stage_label = tk.Label(self.content_frame, text="Инициализация...", font=("Segoe UI", 11, "bold"), fg="#f4f4f5", bg="#18181b", anchor="w")
        self.stage_label.pack(fill="x", padx=12, pady=2)
        
        # Sub-status Label (Small gray text)
        self.sub_label = tk.Label(self.content_frame, text="Пожалуйста, подождите...", font=("Segoe UI", 8), fg="#a1a1aa", bg="#18181b", anchor="w")
        self.sub_label.pack(fill="x", padx=12, pady=(0, 6))
        
        # Progress Bar Canvas
        self.canvas = tk.Canvas(self.content_frame, height=6, bg="#27272a", highlightthickness=0, bd=0)
        self.canvas.pack(fill="x", padx=12, pady=(2, 10))
        self.bar_fill = self.canvas.create_rectangle(0, 0, 0, 6, fill="#00d2ff", width=0)
        
        # Error Details View (hidden by default)
        self.error_frame = tk.Frame(self.content_frame, bg="#18181b")
        # will pack it on error
        
        self.error_label = tk.Label(self.error_frame, text="Детали ошибки:", font=("Segoe UI", 8, "bold"), fg="#f87171", bg="#18181b", anchor="w")
        self.error_label.pack(fill="x", pady=(2, 2))
        
        self.error_text = scrolledtext.ScrolledText(self.error_frame, height=6, font=("Consolas", 8), bg="#27272a", fg="#f87171", bd=0, highlightthickness=0)
        self.error_text.pack(fill="both", expand=True)
        
        # Close Button Frame (hidden by default, shown on error/finish)
        self.btn_frame = tk.Frame(self.content_frame, bg="#18181b")
        self.close_btn = tk.Button(self.btn_frame, text="Закрыть", font=("Segoe UI", 9), bg="#27272a", fg="#f4f4f5", activebackground="#3f3f46", activeforeground="#ffffff", bd=0, padx=10, pady=2, command=self.close_window)
        self.close_btn.pack(side="right")
        
        self.copy_btn = tk.Button(self.btn_frame, text="Копировать ошибку", font=("Segoe UI", 9), bg="#27272a", fg="#f4f4f5", activebackground="#3f3f46", activeforeground="#ffffff", bd=0, padx=10, pady=2, command=self.copy_error)
        self.copy_btn.pack(side="left")
        
        # Width & Positions
        self.width = 330
        self.height = 115
        self.error_height = 270
        self.is_visible = False
        
        # Handle success close timer
        self.close_timer = None
        self.last_status = "idle"
        
        # Start periodic GUI updates
        self.update_gui()

    def update_gui(self):
        """Timer loop that updates Tkinter widgets from shared state."""
        try:
            # Read state thread-safely
            with state.lock:
                is_deploying = state.is_deploying
                progress = state.progress
                stage = state.stage
                sub_status = state.sub_status
                status = state.status
                error_text = state.error_text
                deployment_title = state.deployment_title
            
            if is_deploying:
                if not self.is_visible:
                    # Query monitor coords and calculate position at bottom-right
                    coords = get_target_monitor_coords()
                    x = coords["WorkingX"] + coords["WorkingWidth"] - self.width - 20
                    y = coords["WorkingY"] + coords["WorkingHeight"] - self.height - 20
                    
                    self.root.geometry(f"{self.width}x{self.height}+{x}+{y}")
                    self.root.deiconify()
                    self.is_visible = True
                    self.last_status = "running"
                    
                    # Reset frames
                    self.error_frame.pack_forget()
                    self.btn_frame.pack_forget()
                    self.canvas.pack(fill="x", padx=12, pady=(2, 10))
                    self.canvas.itemconfig(self.bar_fill, fill="#00d2ff")
                
                # Update text labels
                self.title_label.configure(text=deployment_title)
                self.stage_label.configure(text=stage)
                self.sub_label.configure(text=sub_status)
                self.percent_label.configure(text=f"{progress}%")
                
                # Update progress bar length
                canvas_width = self.canvas.winfo_width()
                if canvas_width > 1:
                    fill_width = int(canvas_width * (progress / 100.0))
                    self.canvas.coords(self.bar_fill, 0, 0, fill_width, 6)
                
                # Handle status transitions
                if status == "success" and self.last_status == "running":
                    self.last_status = "success"
                    self.canvas.itemconfig(self.bar_fill, fill="#22c55e")  # Success Green
                    # Auto-close window in 3 seconds
                    self.close_timer = self.root.after(3000, self.close_window)
                    
                elif status == "error" and self.last_status == "running":
                    self.last_status = "error"
                    self.canvas.itemconfig(self.bar_fill, fill="#ef4444")  # Red error bar
                    
                    # Reposition and expand window to fit error details
                    coords = get_target_monitor_coords()
                    x = coords["WorkingX"] + coords["WorkingWidth"] - self.width - 20
                    y = coords["WorkingY"] + coords["WorkingHeight"] - self.error_height - 20
                    
                    self.root.geometry(f"{self.width}x{self.error_height}+{x}+{y}")
                    
                    # Show error components
                    self.error_text.delete("1.0", tk.END)
                    self.error_text.insert(tk.END, error_text)
                    self.error_frame.pack(fill="both", expand=True, padx=12, pady=(2, 6))
                    self.btn_frame.pack(fill="x", padx=12, pady=(2, 10))
            
            else:
                if self.is_visible:
                    self.root.withdraw()
                    self.is_visible = False
                    if self.close_timer:
                        self.root.after_cancel(self.close_timer)
                        self.close_timer = None
        except Exception as e:
            print(f"Error in Tkinter update loop: {e}")
            
        # Schedule next update in 100ms
        self.root.after(100, self.update_gui)

    def close_window(self):
        """Closes the progress window and resets state."""
        if self.close_timer:
            self.root.after_cancel(self.close_timer)
            self.close_timer = None
        
        # Hide window
        self.root.withdraw()
        self.is_visible = False
        
        # Reset deploy state
        state.update(
            is_deploying=False,
            status="idle",
            progress=0,
            stage="Ожидание...",
            sub_status="",
            error_text="",
            deployment_title="POYMAI DEPLOYER",
        )

    def copy_error(self):
        """Copies the contents of the error text box to the system clipboard."""
        try:
            error_content = self.error_text.get("1.0", tk.END).strip()
            self.root.clipboard_clear()
            self.root.clipboard_append(error_content)
            self.copy_btn.configure(text="Скопировано!")
            self.root.after(2000, lambda: self.copy_btn.configure(text="Копировать ошибку"))
        except Exception as e:
            print(f"Error copying to clipboard: {e}")

def run_tray_thread(icon):
    """Background thread to run the pystray tray icon loop."""
    icon.run()


def run_self_test():
    """Validates packaged deployment profiles without opening the tray UI."""
    check_or_create_script()
    for profile_name, profile in DEPLOY_PROFILES.items():
        if not os.path.isdir(profile["folder"]):
            raise FileNotFoundError(
                f"Deploy folder for '{profile_name}' was not found: {profile['folder']}"
            )
        if not os.path.isfile(profile["script"]):
            raise FileNotFoundError(
                f"Deploy script for '{profile_name}' was not found: {profile['script']}"
            )
        if not profile["stage_map"] or max(
            progress for progress, _ in profile["stage_map"].values()
        ) != 100:
            raise RuntimeError(f"Invalid progress map for '{profile_name}'.")
        get_powershell_command(profile["script"])
    print(f"PoymaiDeploy {APP_VERSION} self-test: OK")

def main():
    # Make sure deploy.ps1 template is available on first launch
    check_or_create_script()
    
    # Initialize Tkinter root window
    root = tk.Tk()
    root.title("Poymai Deployer")
    
    # Hide Tkinter root immediately since we only want the borderless popup
    root.withdraw()
    
    # Generate initial play icon
    play_image = create_play_icon(64)
    
    # Define system tray menu
    menu = pystray.Menu(
        pystray.MenuItem('Запустить деплой (Run)', trigger_deploy, default=True),
        pystray.MenuItem('Запустить деплой Telegram', trigger_telegram_deploy),
        pystray.Menu.SEPARATOR,
        pystray.MenuItem('Открыть папку скрипта', open_folder),
        pystray.MenuItem('Запускать вместе с Windows', toggle_startup, checked=is_startup_enabled),
        pystray.Menu.SEPARATOR,
        pystray.MenuItem('Выход', lambda icon, item: [icon.stop(), root.destroy()])
    )
    
    # Initialize and register icon in shared state
    icon = pystray.Icon("PoymaiDeploy", play_image, "Poymai Deployer", menu)
    state.update(pystray_icon=icon)
    
    # Run the tray icon in a separate daemon thread
    tray_thread = threading.Thread(target=run_tray_thread, args=(icon,))
    tray_thread.daemon = True
    tray_thread.start()
    
    # Initialize the custom Progress Window (attaches to root)
    app_window = ProgressWindow(root)
    
    # Run Tkinter mainloop on the main thread (keeps the process alive)
    root.mainloop()

if __name__ == "__main__":
    if "--self-test" in sys.argv:
        run_self_test()
    else:
        main()
