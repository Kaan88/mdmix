#!/usr/bin/env python3
"""
SoundBlaster X G6 GUI
CustomTkinter frontend for soundblaster-x-g6-cli
"""
import queue
import subprocess
import threading
import time
from datetime import datetime

import customtkinter as ctk

try:
    import hid as _hid
    _HID_AVAILABLE = True
except ImportError:
    _HID_AVAILABLE = False

G6_VENDOR_ID  = 0x041E
G6_PRODUCT_ID = 0x3256
G6_HID_IFACE  = 4

CLI = "soundblaster-x-g6-cli"

ctk.set_appearance_mode("dark")
ctk.set_default_color_theme("dark-blue")

WARN_BG  = ("#fff3cd", "#3d3000")
WARN_FG  = ("#7a5500", "#ffcc44")
CON_BG   = "#0d0d0d"
CON_COLS = {
    "cmd":  "#4fc3f7",
    "ok":   "#81c784",
    "err":  "#ef5350",
    "warn": "#ffb74d",
    "dim":  "#616161",
}


# ── Password dialog ───────────────────────────────────────────────────────────

class PasswordDialog(ctk.CTkToplevel):
    def __init__(self, parent, prompt: str = "Enter sudo password:"):
        super().__init__(parent)
        self.title("Authentication Required")
        self.geometry("360x160")
        self.resizable(False, False)
        self.grab_set()
        self.lift()
        self.focus_force()
        self._result: str | None = None

        ctk.CTkLabel(self, text="🔒  " + prompt,
                     font=ctk.CTkFont(size=13)).pack(pady=(20, 8))
        self._entry = ctk.CTkEntry(self, show="●", width=260,
                                   placeholder_text="password")
        self._entry.pack()
        self._entry.focus_set()
        self._entry.bind("<Return>", lambda e: self._ok())

        row = ctk.CTkFrame(self, fg_color="transparent")
        row.pack(pady=14)
        ctk.CTkButton(row, text="OK",     width=100, command=self._ok
                      ).pack(side="left", padx=6)
        ctk.CTkButton(row, text="Cancel", width=100, fg_color="transparent",
                      border_width=1, command=self._cancel
                      ).pack(side="left", padx=6)
        self.protocol("WM_DELETE_WINDOW", self._cancel)

    def _ok(self):
        self._result = self._entry.get()
        self.destroy()

    def _cancel(self):
        self._result = None
        self.destroy()

    @classmethod
    def ask(cls, parent, prompt: str = "Enter sudo password:") -> "str | None":
        dlg = cls(parent, prompt)
        parent.wait_window(dlg)
        return dlg._result


# ── Main window ───────────────────────────────────────────────────────────────

class G6GUI(ctk.CTk):
    def __init__(self):
        super().__init__()
        self.title("SoundBlaster X G6")
        self.geometry("780x820")
        self.minsize(660, 580)
        self.resizable(True, True)

        self._built_tabs: set    = set()
        self._tab_builders: dict = {}
        self._busy               = False
        self._sudo_pw: str | None = None
        self._log_queue: queue.Queue = queue.Queue()
        self._dmesg_proc = None
        self._device_state: dict = {}   # populated by _query_device_state on startup

        self._dry_run      = ctk.BooleanVar(value=False)
        self._auto_reload  = ctk.BooleanVar(value=True)
        self._use_modprobe = ctk.BooleanVar(value=True)
        self._use_sudo     = ctk.BooleanVar(value=True)
        self._settle_ms    = ctk.IntVar(value=600)
        self._show_dmesg   = ctk.BooleanVar(value=False)

        self._build_chrome()
        self.after(0, lambda: self._ensure_tab("Playback"))
        self._drain_log_queue()
        threading.Thread(target=self._query_device_state, daemon=True).start()

    # ── Device state query ────────────────────────────────────────────────────

    def _hid_query(self, payload: bytes) -> "bytes | None":
        """Send a 64-byte read query to the G6 and return the 64-byte response."""
        if not _HID_AVAILABLE:
            return None
        buf = bytearray(64)
        buf[:len(payload)] = payload
        try:
            for info in _hid.enumerate(G6_VENDOR_ID, G6_PRODUCT_ID):
                if info.get("interface_number") == G6_HID_IFACE:
                    dev = _hid.device()
                    dev.open_path(info["path"])
                    dev.set_nonblocking(0)
                    dev.write(b'\x00' + bytes(buf))
                    resp = dev.read(64, timeout_ms=500)
                    dev.close()
                    return bytes(resp) if resp else None
        except Exception as e:
            self._log(f"HID query error: {e}", "err")
        return None

    def _query_device_state(self):
        """Run at startup in a background thread. Queries the device for current state."""
        if not _HID_AVAILABLE:
            self._log("hid module not found — device state unavailable", "dim")
            return

        # Firmware version
        r = self._hid_query(bytes.fromhex("5a070102"))
        if r and len(r) >= 3 and r[2] > 0:
            try:
                fw = r[3:3 + r[2]].decode("ascii").rstrip("\x00")
                self._device_state["firmware"] = fw
            except Exception:
                pass

        # Output: byte[4] 0x02=Speakers, 0x01=Headphones
        r = self._hid_query(bytes.fromhex("5a2c0101"))
        if r and len(r) >= 5:
            self._device_state["output"] = "Speakers" if r[4] == 0x02 else "Headphones"

        # Decoder mode: byte[5] 0x02=Normal, 0x01=Full, 0x00=Night
        # (confirmed Normal=0x02 from capture; Full/Night ordering assumed from CLI enum)
        r = self._hid_query(bytes.fromhex("5a3c0201"))
        if r and len(r) >= 6:
            self._device_state["decoder_mode"] = {
                0x02: "Normal", 0x01: "Full", 0x00: "Night"
            }.get(r[5], "Normal")

        # Volume ring light: byte[5] mirrors write — 0x01=Disabled, 0x00=Enabled
        r = self._hid_query(bytes.fromhex("5a3a020e01"))
        if r and len(r) >= 6:
            self._device_state["ring_enabled"] = (r[5] == 0x00)

        # Direct mode + Scout: byte[4] is a bitmask from query 5a390101
        # bit5 (0x20) is always set in known captures; other bits TBD from more captures
        # byte[4] with nothing active = 0x20; need capture with direct mode ON to confirm bit
        r = self._hid_query(bytes.fromhex("5a390101"))
        if r and len(r) >= 5:
            flags = r[4]
            self._device_state["mode_flags"] = flags
            # Store raw for now; direct mode bit position unconfirmed
            self._device_state["direct_mode_raw"] = flags

        # Speaker/headphone channel config: query 5a390102
        # byte[6]: 0x02=Speakers, 0x04=Headphones (matches output routing)
        # byte[7]: channel count — 0x04=stereo confirmed; 0x06=5.1, 0x08=7.1 assumed
        r = self._hid_query(bytes.fromhex("5a390102"))
        if r and len(r) >= 8:
            ch_map = {0x04: "Stereo", 0x06: "5.1", 0x08: "7.1"}
            self._device_state["channel_config"] = ch_map.get(r[7], f"0x{r[7]:02x}")
            self._device_state["channel_raw"] = r[7]

        self.after(0, self._apply_device_state)

    def _apply_device_state(self):
        """Called on main thread after query completes. Updates any already-built widgets."""
        fw      = self._device_state.get("firmware")
        out     = self._device_state.get("output", "?")
        dec     = self._device_state.get("decoder_mode", "?")
        ring    = self._device_state.get("ring_enabled")
        ch      = self._device_state.get("channel_config", "?")
        flags   = self._device_state.get("mode_flags")
        ring_s  = "on" if ring else "off" if ring is not None else "?"
        flags_s = f"  flags=0x{flags:02x}" if flags is not None else ""
        if fw:
            self.title(f"SoundBlaster X G6  —  fw {fw}")
        self._log(
            f"Device  output={out}  decoder={dec}  ring={ring_s}"
            f"  channels={ch}{flags_s}"
            + (f"  fw={fw}" if fw else ""),
            "dim"
        )

        # Decoder option menu (Playback tab may already be built)
        if hasattr(self, "_decoder_var") and "decoder_mode" in self._device_state:
            self._decoder_var.set(self._device_state["decoder_mode"])

        # Ring switch (Lighting tab may already be built)
        if hasattr(self, "_ring_var") and ring is not None:
            self._ring_var.set(ring)

    # ── Console logging ───────────────────────────────────────────────────────

    def _log(self, text: str, kind: str = "cmd"):
        ts = datetime.now().strftime("%H:%M:%S")
        self._log_queue.put((f"[{ts}] {text}\n", kind))

    def _drain_log_queue(self):
        try:
            while True:
                text, kind = self._log_queue.get_nowait()
                self._console.configure(state="normal")
                self._console.insert("end", text)
                try:
                    tw = self._console._textbox
                    line_start = tw.index("end-2l linestart")
                    line_end   = tw.index("end-1c")
                    tag = f"col_{kind}"
                    if tag not in tw.tag_names():
                        tw.tag_configure(tag, foreground=CON_COLS.get(kind, "#ffffff"))
                    tw.tag_add(tag, line_start, line_end)
                except Exception:
                    pass
                self._console.configure(state="disabled")
                self._console.see("end")
        except queue.Empty:
            pass
        self.after(80, self._drain_log_queue)

    # ── Sudo helpers ──────────────────────────────────────────────────────────

    def _get_sudo_pw(self, force: bool = False) -> "str | None":
        if self._sudo_pw and not force:
            return self._sudo_pw
        pw = PasswordDialog.ask(self, "Enter sudo password:")
        if pw is not None:
            self._sudo_pw = pw
        return pw

    def _sudo_run(self, args: list, pw: str, timeout: int = 15) -> "tuple[bool, str]":
        cmd = ["sudo", "-S"] + args
        try:
            r = subprocess.run(cmd, input=pw + "\n", capture_output=True,
                               text=True, timeout=timeout)
            out = (r.stdout + r.stderr).strip()
            if r.returncode != 0:
                if "incorrect password" in out.lower() or "try again" in out.lower():
                    self._sudo_pw = None
                return False, out
            return True, out
        except subprocess.TimeoutExpired:
            return False, "timed out"
        except FileNotFoundError:
            return False, "sudo not found"

    # ── Command execution ─────────────────────────────────────────────────────

    def run(self, *args, claim: bool = False):
        if self._busy:
            self._log("⏳  A command is already running — please wait.", "warn")
            return
        threading.Thread(
            target=self._run_in_thread, args=(args, claim), daemon=True
        ).start()

    AUDIO_SERVICES = ["wireplumber", "pipewire-pulse", "pipewire"]

    def _systemctl_user(self, action: str) -> bool:
        cmd = ["systemctl", "--user", action] + self.AUDIO_SERVICES
        self._log("$ " + " ".join(cmd), "cmd")
        try:
            r = subprocess.run(cmd, capture_output=True, text=True, timeout=10)
            out = (r.stdout + r.stderr).strip()
            if out:
                for line in out.splitlines():
                    self._log(line, "ok" if r.returncode == 0 else "err")
            return r.returncode == 0
        except Exception as e:
            self._log(f"✗  {e}", "err")
            return False

    def _run_in_thread(self, args: tuple, claim: bool):
        self._busy = True
        dry = self._dry_run.get()
        do_modprobe = claim and not dry and self._auto_reload.get() and self._use_modprobe.get()

        if do_modprobe:
            pw = self._get_sudo_pw()
            if pw is None:
                self._log("✗  Cancelled (no password)", "err")
                self._busy = False
                return
            self._log("⏹  Stopping audio services…", "warn")
            self._systemctl_user("stop")
            time.sleep(0.4)
            self._log("↓  sudo modprobe -r snd_usb_audio", "warn")
            ok, out = self._sudo_run(["modprobe", "-r", "snd_usb_audio"], pw)
            if out:
                self._log(out, "err" if not ok else "dim")
            if not ok:
                self._log("✗  modprobe -r failed — restarting services and aborting", "err")
                self._systemctl_user("start")
                self._busy = False
                return

        cmd = [CLI]
        if dry:
            cmd.append("--dry-run")
        if claim:
            cmd.append("--claim-and-release")
        cmd.extend(str(a) for a in args)
        self._log("$ " + " ".join(cmd), "cmd")
        self._exec_cli(cmd)

        if do_modprobe:
            ms = max(0, self._settle_ms.get())
            if ms > 0:
                self._log(f"⏳  Settling {ms} ms…", "warn")
                time.sleep(ms / 1000)
            pw = self._get_sudo_pw()
            if pw:
                self._log("↑  sudo modprobe snd_usb_audio", "warn")
                ok2, out = self._sudo_run(["modprobe", "snd_usb_audio"], pw)
                if out:
                    self._log(out, "ok" if ok2 else "err")
            self._log("▶  Starting audio services…", "warn")
            self._systemctl_user("start")
            self._log("✓  Done", "ok")

        elif claim and not dry and self._auto_reload.get():
            flag = ("--reload-audio-services"
                    if self._use_sudo.get()
                    else "--reload-audio-services-no-sudo")
            self._log(f"↻  {CLI} {flag}", "warn")
            self._exec_cli([CLI, flag])

        self._busy = False

    def _exec_cli(self, cmd: list) -> bool:
        try:
            r = subprocess.run(cmd, capture_output=True, text=True, timeout=15)
            combined = (r.stdout + r.stderr).strip()
            if combined:
                kind = "ok" if r.returncode == 0 else "err"
                for line in combined.splitlines():
                    self._log(line, kind)
            if r.returncode != 0:
                self._log(f"✗  exit {r.returncode}", "err")
                return False
            return True
        except FileNotFoundError:
            self._log("✗  soundblaster-x-g6-cli not found in $PATH", "err")
            return False
        except subprocess.TimeoutExpired:
            self._log("✗  Command timed out", "err")
            return False

    # ── dmesg tail ────────────────────────────────────────────────────────────

    def _toggle_dmesg(self, *_):
        if self._show_dmesg.get():
            if self._dmesg_proc is None:
                try:
                    self._dmesg_proc = subprocess.Popen(
                        ["dmesg", "-w", "--level=err,warn,info"],
                        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                        text=True, bufsize=1,
                    )
                    threading.Thread(target=self._read_dmesg, daemon=True).start()
                    self._log("dmesg tail started", "dim")
                except FileNotFoundError:
                    self._log("dmesg not available", "dim")
        else:
            if self._dmesg_proc:
                self._dmesg_proc.kill()
                self._dmesg_proc = None
                self._log("dmesg tail stopped", "dim")

    def _read_dmesg(self):
        try:
            for line in self._dmesg_proc.stdout:
                line = line.strip()
                if line:
                    self._log(line, "dim")
        except Exception:
            pass

    # ── Widget helpers ────────────────────────────────────────────────────────

    def _audio_banner(self, parent):
        f = ctk.CTkFrame(parent, fg_color=WARN_BG, corner_radius=6)
        f.pack(fill="x", padx=6, pady=(6, 2))
        ctk.CTkLabel(f,
            text="⚠  [Audio] commands require exclusive USB claim.\n"
                 "   PipeWire stopped → snd_usb_audio removed → command → module + PipeWire restored.",
            text_color=WARN_FG, font=ctk.CTkFont(size=11),
            justify="left", anchor="w",
        ).pack(padx=10, pady=5, anchor="w")

    def _section(self, parent, title: str, badge: str = "") -> ctk.CTkFrame:
        frame = ctk.CTkFrame(parent)
        hdr = ctk.CTkFrame(frame, fg_color="transparent")
        hdr.pack(fill="x", padx=10, pady=(8, 0))
        ctk.CTkLabel(hdr, text=title,
                     font=ctk.CTkFont(size=13, weight="bold"), anchor="w",
                     ).pack(side="left")
        if badge:
            bg = WARN_BG if badge == "[Audio]" else (("#dff0d8", "#1a3a1a"))
            fg = WARN_FG if badge == "[Audio]" else (("#2d6a2d", "#66cc66"))
            ctk.CTkLabel(hdr, text=badge,
                         font=ctk.CTkFont(size=10, weight="bold"),
                         text_color=fg, fg_color=bg, corner_radius=4,
                         ).pack(side="left", padx=6)
        ctk.CTkFrame(frame, height=1, fg_color=("gray65", "gray38")
                     ).pack(fill="x", padx=10, pady=(3, 5))
        return frame

    def _toggle(self, parent, label, flag, padx=10, claim=False):
        row = ctk.CTkFrame(parent, fg_color="transparent")
        row.pack(fill="x", padx=padx, pady=2)
        ctk.CTkLabel(row, text=label, width=230, anchor="w").pack(side="left")
        for value in ("Enabled", "Disabled"):
            ctk.CTkButton(
                row, text=value, width=95,
                fg_color="transparent", border_width=1,
                command=lambda v=value: self.run(f"--{flag}", v, claim=claim),
            ).pack(side="left", padx=2)

    def _slider(self, parent, label, flag, default=0,
                lo=0, hi=100, step=1, padx=10, claim=False):
        row = ctk.CTkFrame(parent, fg_color="transparent")
        row.pack(fill="x", padx=padx, pady=2)
        ctk.CTkLabel(row, text=label, width=230, anchor="w").pack(side="left")
        var = ctk.IntVar(value=default)
        ctk.CTkLabel(row, textvariable=var, width=36, anchor="e").pack(side="right")
        sl = ctk.CTkSlider(row, from_=lo, to=hi, variable=var,
                           number_of_steps=max(1, int((hi - lo) / step)))
        sl.pack(side="left", fill="x", expand=True, padx=4)
        sl.bind("<ButtonRelease-1>",
                lambda e, f=flag, v=var, c=claim:
                    self.run(f"--{f}", int(v.get()), claim=c))
        return var

    def _option(self, parent, label, flag, values, default=None, padx=10, claim=False):
        row = ctk.CTkFrame(parent, fg_color="transparent")
        row.pack(fill="x", padx=padx, pady=2)
        ctk.CTkLabel(row, text=label, width=230, anchor="w").pack(side="left")
        var = ctk.StringVar(value=default or values[0])
        ctk.CTkOptionMenu(row, values=values, variable=var, width=270,
                          command=lambda v: self.run(f"--{flag}", v, claim=claim),
                          ).pack(side="left", padx=4)
        return var

    def _channels(self, parent, label, flag, padx=10, claim=False):
        row = ctk.CTkFrame(parent, fg_color="transparent")
        row.pack(fill="x", padx=padx, pady=2)
        ctk.CTkLabel(row, text=label, width=230, anchor="w").pack(side="left")
        for value in ("Both", "Left", "Right"):
            ctk.CTkButton(
                row, text=value, width=75,
                fg_color="transparent", border_width=1,
                command=lambda v=value: self.run(f"--{flag}", v, claim=claim),
            ).pack(side="left", padx=2)

    def _mixer_source(self, parent, label, base, padx=14):
        ctk.CTkLabel(parent, text=label,
                     font=ctk.CTkFont(size=12, weight="bold"), anchor="w",
                     ).pack(anchor="w", padx=padx, pady=(7, 0))
        self._toggle(parent,   "  Mute",     f"{base}-mute",             padx=padx+10, claim=True)
        self._slider(parent,   "  Volume",   f"{base}-volume",           lo=0, hi=100, step=10, padx=padx+10, claim=True)
        self._channels(parent, "  Channels", f"{base}-volume-channels",  padx=padx+10, claim=True)

    # ── Chrome ────────────────────────────────────────────────────────────────

    def _build_chrome(self):
        hdr = ctk.CTkFrame(self, height=54, corner_radius=0)
        hdr.pack(fill="x")
        hdr.pack_propagate(False)
        ctk.CTkLabel(hdr, text="SoundBlaster X G6",
                     font=ctk.CTkFont(size=17, weight="bold"),
                     ).pack(side="left", padx=16)
        ctk.CTkLabel(hdr, text="Output:").pack(side="left", padx=(20, 4))
        for val in ("Speakers", "Headphones"):
            ctk.CTkButton(
                hdr, text=val, width=95,
                fg_color="transparent", border_width=1,
                command=lambda v=val: self.run("--set-output", v),
            ).pack(side="left", padx=2)
        ctk.CTkButton(hdr, text="Toggle", width=72,
                      command=lambda: self.run("--toggle-output"),
                      ).pack(side="left", padx=7)
        ctk.CTkCheckBox(hdr, text="Dry Run", variable=self._dry_run, width=95,
                        ).pack(side="right", padx=14)

        cb = ctk.CTkFrame(self, height=38, corner_radius=0, fg_color=WARN_BG)
        cb.pack(fill="x")
        cb.pack_propagate(False)
        ctk.CTkLabel(cb, text="[Audio]:",
                     text_color=WARN_FG, font=ctk.CTkFont(size=11, weight="bold"),
                     ).pack(side="left", padx=10)
        ctk.CTkCheckBox(cb, text="Auto-restore audio",
                        variable=self._auto_reload,
                        text_color=WARN_FG, font=ctk.CTkFont(size=11),
                        ).pack(side="left", padx=6)
        ctk.CTkCheckBox(cb, text="modprobe (recommended)",
                        variable=self._use_modprobe,
                        text_color=WARN_FG, font=ctk.CTkFont(size=11),
                        ).pack(side="left", padx=6)
        ctk.CTkCheckBox(cb, text="sudo",
                        variable=self._use_sudo,
                        text_color=WARN_FG, font=ctk.CTkFont(size=11),
                        ).pack(side="left", padx=6)
        ctk.CTkLabel(cb, text="Settle ms:",
                     text_color=WARN_FG, font=ctk.CTkFont(size=11),
                     ).pack(side="left", padx=(12, 2))
        ctk.CTkSlider(cb, from_=0, to=2000, variable=self._settle_ms,
                      number_of_steps=40, width=100,
                      ).pack(side="left")
        ctk.CTkLabel(cb, textvariable=self._settle_ms, width=36,
                     text_color=WARN_FG, font=ctk.CTkFont(size=11),
                     ).pack(side="left", padx=2)
        ctk.CTkButton(cb, text="Clear password", width=110,
                      fg_color="transparent", border_width=1,
                      text_color=WARN_FG, border_color=WARN_FG,
                      font=ctk.CTkFont(size=11),
                      command=lambda: setattr(self, "_sudo_pw", None) or
                                      self._log("Sudo password cleared", "warn"),
                      ).pack(side="right", padx=10)

        main = ctk.CTkFrame(self, fg_color="transparent")
        main.pack(fill="both", expand=True)
        main.rowconfigure(0, weight=3)
        main.rowconfigure(1, weight=0)
        main.rowconfigure(2, weight=1, minsize=120)
        main.columnconfigure(0, weight=1)

        self._tabs = ctk.CTkTabview(main)
        self._tabs.grid(row=0, column=0, sticky="nsew", padx=10, pady=(6, 0))
        for name, fn in [
            ("Playback",    self._build_tab_playback),
            ("SBX Effects", self._build_tab_sbx),
            ("Recording",   self._build_tab_recording),
            ("Mixer",       self._build_tab_mixer),
            ("Lighting",    self._build_tab_lighting),
            ("System",      self._build_tab_system),
        ]:
            self._tabs.add(name)
            self._tab_builders[name] = fn
        self._tabs.configure(command=lambda: self._ensure_tab(self._tabs.get()))

        div = ctk.CTkFrame(main, height=28, fg_color=("gray85", "gray20"),
                           corner_radius=0)
        div.grid(row=1, column=0, sticky="ew")
        ctk.CTkLabel(div, text="  OUTPUT", anchor="w",
                     font=ctk.CTkFont(size=10, weight="bold"),
                     text_color=("gray50", "gray55"),
                     ).pack(side="left", padx=8)
        ctk.CTkButton(div, text="Clear", width=55, height=20,
                      fg_color="transparent", border_width=1,
                      font=ctk.CTkFont(size=10),
                      command=self._clear_console,
                      ).pack(side="right", padx=8, pady=4)
        ctk.CTkCheckBox(div, text="dmesg",
                        variable=self._show_dmesg,
                        font=ctk.CTkFont(size=10),
                        command=self._toggle_dmesg,
                        ).pack(side="right", padx=4)

        self._console = ctk.CTkTextbox(
            main, state="disabled",
            fg_color=CON_BG, text_color="#cccccc",
            font=ctk.CTkFont(family="monospace", size=11),
            corner_radius=0, wrap="word",
        )
        self._console.grid(row=2, column=0, sticky="nsew")

    def _ensure_tab(self, name: str):
        if name not in self._built_tabs:
            self._built_tabs.add(name)
            self._tab_builders[name](self._tabs.tab(name))

    def _clear_console(self):
        self._console.configure(state="normal")
        self._console.delete("1.0", "end")
        self._console.configure(state="disabled")

    # ── Tabs ──────────────────────────────────────────────────────────────────

    def _build_tab_playback(self, parent):
        sf = ctk.CTkScrollableFrame(parent)
        sf.pack(fill="both", expand=True)

        hid = self._section(sf, "Mode & Filter", badge="[HID]")
        hid.pack(fill="x", padx=4, pady=4)
        self._toggle(hid, "Direct Mode",           "playback-direct-mode")
        self._toggle(hid, "SPDIF Out Direct Mode", "playback-spdif-out-direct-mode")
        self._option(hid, "DAC Filter",            "playback-filter", [
            "FAST_ROLL_OFF_MINIMUM_PHASE", "SLOW_ROLL_OFF_MINIMUM_PHASE",
            "FAST_ROLL_OFF_LINEAR_PHASE",  "SLOW_ROLL_OFF_LINEAR_PHASE",
        ])
        self._decoder_var = self._option(hid, "Decoder Mode", "decoder-mode", ["Normal", "Full", "Night"])

        self._audio_banner(sf)

        vol = self._section(sf, "Volume & Mute", badge="[Audio]")
        vol.pack(fill="x", padx=4, pady=4)
        self._toggle(vol, "Mute Playback",     "playback-mute",           claim=True)
        self._slider(vol, "Volume (0–100)",    "playback-volume",          claim=True)
        self._channels(vol, "Volume Channels", "playback-volume-channels", claim=True)

        layout = self._section(sf, "Speaker / Headphone Layout", badge="[Audio]")
        layout.pack(fill="x", padx=4, pady=4)
        for device, flags in [
            ("Speakers",   ["playback-speakers-to-stereo",
                             "playback-speakers-to-5-1",
                             "playback-speakers-to-7-1"]),
            ("Headphones", ["playback-headphones-to-stereo",
                             "playback-headphones-to-5-1",
                             "playback-headphones-to-7-1"]),
        ]:
            row = ctk.CTkFrame(layout, fg_color="transparent")
            row.pack(fill="x", padx=10, pady=2)
            ctk.CTkLabel(row, text=device, width=230, anchor="w").pack(side="left")
            for lbl, flag in zip(["Stereo", "5.1", "7.1"], flags):
                ctk.CTkButton(row, text=lbl, width=75,
                              command=lambda f=flag: self.run(f"--{f}", claim=True),
                              ).pack(side="left", padx=3)

    def _build_tab_sbx(self, parent):
        sf = ctk.CTkScrollableFrame(parent)
        sf.pack(fill="both", expand=True)
        for name, tf, vf, special in [
            ("Surround",     "sbx-surround",     "sbx-surround-value",     None),
            ("Crystalizer",  "sbx-crystalizer",  "sbx-crystalizer-value",  None),
            ("Bass Boost",   "sbx-bass",         "set-bass-value",         None),
            ("Smart Volume", "sbx-smart-volume", "sbx-smart-volume-value", ["Night", "Loud"]),
            ("Dialog Plus",  "sbx-dialog-plus",  "sbx-dialog-plus-value",  None),
        ]:
            s = self._section(sf, name, badge="[HID]")
            s.pack(fill="x", padx=4, pady=4)
            self._toggle(s, "Enable",            tf)
            self._slider(s, "Intensity (0–100)", vf)
            if special:
                self._option(s, "Special Mode", "sbx-smart-volume-special-value", special)

    def _build_tab_recording(self, parent):
        sf = ctk.CTkScrollableFrame(parent)
        sf.pack(fill="both", expand=True)

        vc = self._section(sf, "Voice Clarity", badge="[HID]")
        vc.pack(fill="x", padx=4, pady=4)
        self._option(vc, "Mic Boost (dB)", "recording-mic-boost-db", ["0", "10", "20", "30"])
        self._toggle(vc, "Enable Voice Clarity",    "recording-voice-clarity")
        self._slider(vc, "Noise Reduction (0–100)", "recording-voice-clarity-noise-reduction",
                     lo=0, hi=100, step=20)
        self._toggle(vc, "AEC",          "recording-voice-clarity-aec")
        self._toggle(vc, "Smart Volume", "recording-voice-clarity-smart-volume")
        self._toggle(vc, "Mic EQ",       "recording-voice-clarity-mic-eq")
        self._option(vc, "Mic EQ Preset", "recording-voice-clarity-mic-eq-preset",
                     [f"PRESET_{i}" for i in range(1, 11)] + ["PRESET_DM_1"])

        self._audio_banner(sf)

        mic = self._section(sf, "Microphone Recording", badge="[Audio]")
        mic.pack(fill="x", padx=4, pady=4)
        self._toggle(mic, "Mute",       "recording-mute",                         claim=True)
        self._slider(mic, "Volume",     "recording-mic-recording-volume",          lo=0, hi=100, step=10, claim=True)
        self._channels(mic, "Channels", "recording-mic-recording-volume-channels", claim=True)

        mon = self._section(sf, "Microphone Monitoring", badge="[Audio]")
        mon.pack(fill="x", padx=4, pady=4)
        self._toggle(mon, "Mute",       "recording-mic-monitoring-mute",           claim=True)
        self._slider(mon, "Volume",     "recording-mic-monitoring-volume",          lo=0, hi=100, step=10, claim=True)
        self._channels(mon, "Channels", "recording-mic-monitoring-volume-channels", claim=True)

    def _build_tab_mixer(self, parent):
        sf = ctk.CTkScrollableFrame(parent)
        sf.pack(fill="both", expand=True)
        self._audio_banner(sf)

        pb = self._section(sf, "Playback Mix", badge="[Audio]")
        pb.pack(fill="x", padx=4, pady=4)
        self._toggle(pb, "Mute Playback Mix", "mixer-playback-mute", claim=True)

        mon = self._section(sf, "Monitoring", badge="[Audio]")
        mon.pack(fill="x", padx=4, pady=4)
        for label, key in [("Line In", "line-in"), ("External Mic", "external-mic"),
                            ("SPDIF In", "spdif-in")]:
            self._mixer_source(mon, label, f"mixer-monitoring-{key}")

        rec = self._section(sf, "Recording Sources", badge="[Audio]")
        rec.pack(fill="x", padx=4, pady=4)
        for label, key in [("Line In", "line-in"), ("External Mic", "external-mic"),
                            ("SPDIF In", "spdif-in"), ("What U Hear", "what-u-hear")]:
            self._mixer_source(rec, label, f"mixer-recording-{key}")

    def _build_tab_lighting(self, parent):
        sf = ctk.CTkScrollableFrame(parent)
        sf.pack(fill="both", expand=True)

        s = self._section(sf, "LED Lighting", badge="[HID]")
        s.pack(fill="x", padx=4, pady=4)

        # RGB accent light
        ctk.CTkButton(s, text="Disable RGB Light",
                      fg_color=("#c0392b", "#8b1a1a"), hover_color=("#a93226", "#6e1414"),
                      command=lambda: self.run("--lighting-disable"),
                      ).pack(anchor="w", padx=10, pady=(4, 2))

        prev_row = ctk.CTkFrame(s, fg_color="transparent")
        prev_row.pack(fill="x", padx=10, pady=(4, 6))
        ctk.CTkLabel(prev_row, text="Custom RGB Color:",
                     width=230, anchor="w").pack(side="left")
        self._preview = ctk.CTkFrame(prev_row, width=52, height=34,
                                     corner_radius=7, fg_color="#ff0000")
        self._preview.pack(side="left", padx=8)
        ctk.CTkButton(prev_row, text="Apply Color", width=100,
                      command=self._apply_rgb).pack(side="left", padx=4)

        self._r = ctk.IntVar(value=255)
        self._g = ctk.IntVar(value=0)
        self._b = ctk.IntVar(value=0)
        for ch, var in [("R", self._r), ("G", self._g), ("B", self._b)]:
            row = ctk.CTkFrame(s, fg_color="transparent")
            row.pack(fill="x", padx=10, pady=2)
            ctk.CTkLabel(row, text=ch, width=20, anchor="w").pack(side="left")
            ctk.CTkLabel(row, textvariable=var, width=36, anchor="e").pack(side="right")
            sl = ctk.CTkSlider(row, from_=0, to=255, variable=var, number_of_steps=255,
                               command=lambda _: self._update_preview())
            sl.pack(side="left", fill="x", expand=True, padx=4)
            sl.bind("<ButtonRelease-1>", lambda e: self._apply_rgb())
        self._update_preview()

        # Volume ring light (separate LED)
        ctk.CTkFrame(s, height=1, fg_color=("gray65", "gray38")
                     ).pack(fill="x", padx=10, pady=(10, 5))
        ring_row = ctk.CTkFrame(s, fg_color="transparent")
        ring_row.pack(fill="x", padx=10, pady=(2, 8))
        ctk.CTkLabel(ring_row, text="Volume Ring Light",
                     width=230, anchor="w").pack(side="left")
        self._ring_var = ctk.BooleanVar(value=self._device_state.get("ring_enabled", True))
        def _ring_toggled():
            v = "Enabled" if self._ring_var.get() else "Disabled"
            self.run("--lighting-volume-ring", v)
        ctk.CTkSwitch(
            ring_row, text="", variable=self._ring_var,
            command=_ring_toggled, width=50,
        ).pack(side="left", padx=4)

    def _build_tab_system(self, parent):
        sf = ctk.CTkScrollableFrame(parent)
        sf.pack(fill="both", expand=True)

        rec = self._section(sf, "Manual Recovery")
        rec.pack(fill="x", padx=4, pady=4)
        ctk.CTkLabel(rec, anchor="w", justify="left", font=ctk.CTkFont(size=11),
                     text=(
                         "With 'modprobe (recommended)', the full sequence runs automatically:\n"
                         "stop PipeWire → modprobe -r → command → modprobe → start PipeWire\n\n"
                         "PipeWire must be stopped first or modprobe -r will fail with\n"
                         "'Module snd_usb_audio is in use'.\n\n"
                         "If still stuck, run the steps below manually in order:"
                     )).pack(padx=10, pady=(2, 8), anchor="w")

        for label, fn in [
            ("1 · systemctl --user stop pipewire pipewire-pulse wireplumber",
                lambda: threading.Thread(target=self._systemctl_user, args=("stop",), daemon=True).start()),
            ("2 · sudo modprobe -r snd_usb_audio",
                lambda: self._sudo_cmd(["modprobe", "-r", "snd_usb_audio"])),
            ("3 · sudo modprobe snd_usb_audio",
                lambda: self._sudo_cmd(["modprobe", "snd_usb_audio"])),
            ("4 · systemctl --user start pipewire pipewire-pulse wireplumber",
                lambda: threading.Thread(target=self._systemctl_user, args=("start",), daemon=True).start()),
            ("— or — Reload audio services (sudo)",
                lambda: self.run("--reload-audio-services")),
        ]:
            ctk.CTkButton(rec, text=label, anchor="w", command=fn,
                          ).pack(fill="x", padx=10, pady=2)

        mc = self._section(sf, "Device Claim (manual)")
        mc.pack(fill="x", padx=4, pady=4)
        ctk.CTkLabel(mc, anchor="w", justify="left", font=ctk.CTkFont(size=11),
                     text="Standalone claim & release without any Audio command."
                     ).pack(padx=10, pady=(2, 6), anchor="w")
        ctk.CTkButton(mc, text="Claim & Release",
                      fg_color=WARN_BG, text_color=WARN_FG,
                      hover_color=("#ffe080", "#5a4800"),
                      command=lambda: self.run("--claim-and-release"),
                      ).pack(anchor="w", padx=10, pady=(0, 8))

    # ── Helpers ───────────────────────────────────────────────────────────────

    def _sudo_cmd(self, args: list):
        def _do():
            pw = self._get_sudo_pw()
            if pw is None:
                self._log("✗  Cancelled", "err")
                return
            self._log("$ sudo " + " ".join(args), "cmd")
            ok, out = self._sudo_run(args, pw)
            if out:
                self._log(out, "ok" if ok else "err")
            if ok:
                self._log("✓  Done", "ok")
        threading.Thread(target=_do, daemon=True).start()

    def _update_preview(self):
        try:
            self._preview.configure(
                fg_color=f"#{self._r.get():02x}{self._g.get():02x}{self._b.get():02x}")
        except AttributeError:
            pass

    def _apply_rgb(self):
        self._update_preview()
        self.run("--lighting-rgb", self._r.get(), self._g.get(), self._b.get())


if __name__ == "__main__":
    app = G6GUI()
    app.mainloop()
