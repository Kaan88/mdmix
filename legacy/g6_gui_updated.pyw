#!/usr/bin/env python3
"""
SoundBlaster X G6 GUI
CustomTkinter frontend for soundblaster-x-g6-cli
"""
import platform
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
        self._platform = platform.system()
        self._is_linux = self._platform == "Linux"
        self._is_windows = self._platform == "Windows"
        self._layout_state: dict = {"Speakers": "?", "Headphones": "?"}  # hidden until reliable readback exists

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

        r = self._hid_query(bytes.fromhex("5a070102"))
        if r and len(r) >= 3 and r[2] > 0:
            try:
                self._device_state["firmware"] = r[3:3 + r[2]].decode("ascii").rstrip("\x00")
            except Exception:
                pass

        r = self._hid_query(bytes.fromhex("5a2c0101"))
        if r and len(r) >= 5:
            self._device_state["output"] = {0x02: "Speakers", 0x04: "Headphones"}.get(r[4], f"0x{r[4]:02x}")

        r = self._hid_query(bytes.fromhex("5a020a398100"))
        if not r:
            r = self._hid_query(bytes.fromhex("5a390101"))
        if r and len(r) >= 6 and (r[:4] == bytes.fromhex("5a390501") or r[:4] == bytes.fromhex("5a390902")):
            flags_a, flags_b = r[4], r[5]
            self._device_state["mode_flags_a"] = flags_a
            self._device_state["mode_flags_b"] = flags_b
            self._device_state["direct_mode"] = bool(flags_a & 0x20)
            self._device_state["spdif_direct"] = bool(flags_b & 0x20)
            # Startup captures with both LEDs on/off show this bit is inverted for the
            # volume ring: bit set means ring off, bit clear means ring on.
            self._device_state["ring_enabled"] = not bool(flags_b & 0x40)

        r = self._hid_query(bytes.fromhex("5a1103019702"))
        if r and len(r) >= 11 and r[:7] == bytes.fromhex("5a110801009702"):
            dec_raw = int.from_bytes(r[7:11], "little")
            self._device_state["decoder_mode"] = {
                0x3f800000: "Full",
                0x40000000: "Normal",
                0x40400000: "Night",
            }.get(dec_raw, f"0x{dec_raw:08x}")

        r = self._hid_query(bytes.fromhex("5a6c0101"))
        if r and len(r) >= 5 and r[:4] == bytes.fromhex("5a6c0301"):
            self._device_state["dac_filter"] = {
                0x01: "Fast Roll-off / Minimum Phase",
                0x02: "Slow Roll-off / Minimum Phase",
                0x04: "Fast Roll-off / Linear Phase",
                0x05: "Slow Roll-off / Linear Phase",
            }.get(r[4], f"0x{r[4]:02x}")

        r = self._hid_query(bytes.fromhex("5a020a3a00060100"))
        if not r:
            r = self._hid_query(bytes.fromhex("5a3a020701"))
        if r and len(r) >= 5 and r[:4] == bytes.fromhex("5a3a0207"):
            # Full startup captures with both LEDs on/off show this status byte is
            # inverted from the earlier toggle-only assumption: 0x01 = on, 0x00 = off.
            self._device_state["rgb_enabled"] = (r[4] == 0x01)

        # The dedicated 0x3a ring query has not been reliable in captures, so only use
        # it as a last-resort fallback. The byte appears to follow the same 0x01=on,
        # 0x00=off convention as the RGB status query.
        if "ring_enabled" not in self._device_state:
            r = self._hid_query(bytes.fromhex("5a020a3a000e0100"))
            if not r:
                r = self._hid_query(bytes.fromhex("5a3a021001"))
            if r and len(r) >= 5 and r[:4] == bytes.fromhex("5a3a0210"):
                self._device_state["ring_enabled"] = (r[4] == 0x01)

        self.after(0, self._apply_device_state)


    def _refresh_device_state_async(self):
        threading.Thread(target=self._query_device_state, daemon=True).start()

    def _apply_device_state(self):
        """Called on main thread after query completes. Updates any already-built widgets."""
        fw = self._device_state.get("firmware")
        if fw:
            self.title(f"SoundBlaster X G6  —  fw {fw}")

        if hasattr(self, "_decoder_var") and "decoder_mode" in self._device_state:
            self._decoder_var.set(self._device_state["decoder_mode"])
        if hasattr(self, "_filter_var") and "dac_filter" in self._device_state:
            self._filter_var.set(self._device_state["dac_filter"])
        if hasattr(self, "_rgb_var") and "rgb_enabled" in self._device_state:
            self._rgb_var.set(self._device_state["rgb_enabled"])
        if hasattr(self, "_ring_var") and "ring_enabled" in self._device_state:
            self._ring_var.set(self._device_state["ring_enabled"])
        if hasattr(self, "_direct_var") and "direct_mode" in self._device_state:
            self._direct_var.set(self._device_state["direct_mode"])
        if hasattr(self, "_spdif_direct_var") and "spdif_direct" in self._device_state:
            self._spdif_direct_var.set(self._device_state["spdif_direct"])

        self._refresh_status_cards()

        fa = self._device_state.get("mode_flags_a")
        fb = self._device_state.get("mode_flags_b")
        flags_s = f"  flags=0x{fa:02x}/0x{fb:02x}" if fa is not None and fb is not None else ""
        self._log(
            "Device  output={output}  decoder={decoder}  filter={filt}  direct={direct}  spdif-direct={spdif}  rgb={rgb}  ring={ring}{flags}{fw}".format(
                output=self._device_state.get("output", "?"),
                decoder=self._device_state.get("decoder_mode", "?"),
                filt=self._device_state.get("dac_filter", "?"),
                direct="on" if self._device_state.get("direct_mode") else "off" if "direct_mode" in self._device_state else "?",
                spdif="on" if self._device_state.get("spdif_direct") else "off" if "spdif_direct" in self._device_state else "?",
                rgb="on" if self._device_state.get("rgb_enabled") else "off" if "rgb_enabled" in self._device_state else "?",
                ring="on" if self._device_state.get("ring_enabled") else "off" if "ring_enabled" in self._device_state else "?",
                flags=flags_s,
                fw=(f"  fw={fw}" if fw else ""),
            ),
            "dim"
        )

    def _refresh_status_cards(self):
        if not hasattr(self, "_status_vars"):
            return
        current_output = self._device_state.get("output", "?")
        self._status_vars["output"].set(str(current_output))
        self._status_vars["decoder"].set(str(self._device_state.get("decoder_mode", "?")))
        self._status_vars["filter"].set(str(self._device_state.get("dac_filter", "?")))
        self._status_vars["direct"].set("On" if self._device_state.get("direct_mode") else "Off" if "direct_mode" in self._device_state else "?")
        self._status_vars["spdif"].set("On" if self._device_state.get("spdif_direct") else "Off" if "spdif_direct" in self._device_state else "?")
        self._status_vars["rgb"].set("On" if self._device_state.get("rgb_enabled") else "Off" if "rgb_enabled" in self._device_state else "?")
        self._status_vars["ring"].set("On" if self._device_state.get("ring_enabled") else "Off" if "ring_enabled" in self._device_state else "?")
        self._status_vars["fw"].set(str(self._device_state.get("firmware", "?")))

    def _state_set(self, **kwargs):
        self._device_state.update(kwargs)
        self._apply_device_state()

    def _effective_claim(self, claim: bool) -> bool:
        return bool(claim and self._is_linux)

    def _audio_badge(self) -> str:
        return "[Audio · Linux]" if self._is_linux else "[Audio]"

    def _run_async(self, *args, claim=False, on_success=None):
        claim = self._effective_claim(claim)
        def _do():
            if self._busy:
                self._log("⏳  A command is already running — please wait.", "warn")
                return
            ok = self._run_in_thread(args, claim)
            if ok and on_success:
                self.after(0, on_success)
        threading.Thread(target=_do, daemon=True).start()

    def _set_output(self, value: str):
        self._run_async("--set-output", value, on_success=lambda: self._state_set(output=value))

    def _set_direct_mode(self, enabled: bool):
        def _done():
            updates = {"direct_mode": enabled}
            if enabled:
                updates["spdif_direct"] = False
                if hasattr(self, "_spdif_direct_var"):
                    self._spdif_direct_var.set(False)
            self._state_set(**updates)

        def _do():
            if self._busy:
                self._log("⏳  A command is already running — please wait.", "warn")
                return
            if enabled and self._device_state.get("spdif_direct"):
                if not self._run_in_thread(("--playback-spdif-out-direct-mode", "Disabled"), False):
                    return
            if self._run_in_thread(("--playback-direct-mode", "Enabled" if enabled else "Disabled"), False):
                self.after(0, _done)

        threading.Thread(target=_do, daemon=True).start()

    def _set_spdif_direct(self, enabled: bool):
        def _done():
            updates = {"spdif_direct": enabled}
            if enabled:
                updates["direct_mode"] = False
                if hasattr(self, "_direct_var"):
                    self._direct_var.set(False)
            self._state_set(**updates)

        def _do():
            if self._busy:
                self._log("⏳  A command is already running — please wait.", "warn")
                return
            if enabled and self._device_state.get("direct_mode"):
                if not self._run_in_thread(("--playback-direct-mode", "Disabled"), False):
                    return
            if self._run_in_thread(("--playback-spdif-out-direct-mode", "Enabled" if enabled else "Disabled"), False):
                self.after(0, _done)

        threading.Thread(target=_do, daemon=True).start()

    def _set_decoder_mode(self, value: str):
        self._run_async("--decoder-mode", value, on_success=lambda: self._state_set(decoder_mode=value))

    def _set_filter(self, value: str):
        cli_value = {
            "Fast Roll-off / Minimum Phase": "FAST_ROLL_OFF_MINIMUM_PHASE",
            "Slow Roll-off / Minimum Phase": "SLOW_ROLL_OFF_MINIMUM_PHASE",
            "Fast Roll-off / Linear Phase": "FAST_ROLL_OFF_LINEAR_PHASE",
            "Slow Roll-off / Linear Phase": "SLOW_ROLL_OFF_LINEAR_PHASE",
        }[value]
        self._run_async("--playback-filter", cli_value, on_success=lambda: self._state_set(dac_filter=value))

    def _set_layout(self, output: str, layout: str):
        flag_map = {
            ("Speakers", "Stereo"): "--playback-speakers-to-stereo",
            ("Speakers", "5.1"): "--playback-speakers-to-5-1",
            ("Speakers", "7.1"): "--playback-speakers-to-7-1",
            ("Headphones", "Stereo"): "--playback-headphones-to-stereo",
            ("Headphones", "5.1"): "--playback-headphones-to-5-1",
            ("Headphones", "7.1"): "--playback-headphones-to-7-1",
        }
        def _done():
            self._layout_state[output] = layout
            if hasattr(self, f"_{output.lower()}_layout_selector"):
                try:
                    getattr(self, f"_{output.lower()}_layout_selector").set(layout)
                except Exception:
                    pass
            self._refresh_status_cards()
        self._run_async(flag_map[(output, layout)], claim=True, on_success=_done)

    def _set_rgb_enabled(self, enabled: bool):
        if enabled:
            self._run_async("--lighting-rgb", self._r.get(), self._g.get(), self._b.get(),
                            on_success=lambda: self._state_set(rgb_enabled=True))
        else:
            self._run_async("--lighting-disable", on_success=lambda: self._state_set(rgb_enabled=False))

    def _set_ring_enabled(self, enabled: bool):
        self._run_async("--lighting-volume-ring", "Enabled" if enabled else "Disabled",
                        on_success=lambda: self._state_set(ring_enabled=enabled))
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
        if not self._is_linux:
            self._log("Audio service control is Linux-only", "dim")
            return False
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
        claim = self._effective_claim(claim)
        dry = self._dry_run.get()
        do_modprobe = claim and not dry and self._auto_reload.get() and self._use_modprobe.get()

        if do_modprobe:
            pw = self._get_sudo_pw()
            if pw is None:
                self._log("✗  Cancelled (no password)", "err")
                self._busy = False
                return False
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
                return False

        cmd = [CLI]
        if dry:
            cmd.append("--dry-run")
        if claim:
            cmd.append("--claim-and-release")
        cmd.extend(str(a) for a in args)
        self._log("$ " + " ".join(cmd), "cmd")
        ok_main = self._exec_cli(cmd)

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
        return ok_main

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
        if not self._is_linux:
            self._show_dmesg.set(False)
            self._log("dmesg tail is Linux-only", "dim")
            return
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
        if self._is_linux:
            text = ("⚠  Linux audio commands may need exclusive USB claim.\n"
                    "   PipeWire stopped → snd_usb_audio removed → command → module + PipeWire restored.")
        else:
            text = ("ℹ  Driver recovery controls are Linux-only.\n"
                    "   On Windows, playback/recording commands run without the Linux claim/release flow.")
        ctk.CTkLabel(f,
            text=text,
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
                command=lambda v=val: self._set_output(v),
            ).pack(side="left", padx=2)
        ctk.CTkButton(hdr, text="Toggle", width=72,
                      command=lambda: self.run("--toggle-output"),
                      ).pack(side="left", padx=7)
        ctk.CTkCheckBox(hdr, text="Dry Run", variable=self._dry_run, width=95,
                        ).pack(side="right", padx=14)

        cb = ctk.CTkFrame(self, height=38, corner_radius=0, fg_color=WARN_BG)
        cb.pack(fill="x")
        cb.pack_propagate(False)
        if self._is_linux:
            ctk.CTkLabel(cb, text="[Audio · Linux]:",
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
        else:
            ctk.CTkLabel(cb, text="Windows mode: Linux claim/release and modprobe recovery are hidden.",
                         text_color=WARN_FG, font=ctk.CTkFont(size=11),
                         ).pack(side="left", padx=10)
            ctk.CTkButton(cb, text="Refresh Status", width=110,
                          fg_color="transparent", border_width=1,
                          text_color=WARN_FG, border_color=WARN_FG,
                          font=ctk.CTkFont(size=11),
                          command=self._refresh_device_state_async,
                          ).pack(side="right", padx=10)

        status = ctk.CTkFrame(self, fg_color=("gray90", "gray16"), corner_radius=0)
        status.pack(fill="x")
        self._status_vars = {k: ctk.StringVar(value="?") for k in ("output", "decoder", "filter", "direct", "spdif", "rgb", "ring", "fw")}
        for title, key in [("Output", "output"), ("Decoder", "decoder"), ("Filter", "filter"),
                           ("Direct", "direct"), ("SPDIF Direct", "spdif"), ("RGB", "rgb"),
                           ("Ring", "ring"), ("Firmware", "fw")]:
            card = ctk.CTkFrame(status, fg_color="transparent")
            card.pack(side="left", padx=8, pady=4)
            ctk.CTkLabel(card, text=title, font=ctk.CTkFont(size=10), text_color=("gray45", "gray70")).pack(anchor="w")
            ctk.CTkLabel(card, textvariable=self._status_vars[key], font=ctk.CTkFont(size=12, weight="bold")).pack(anchor="w")
        ctk.CTkButton(status, text="Refresh", width=72, height=26,
                      command=self._refresh_device_state_async).pack(side="right", padx=10, pady=6)

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
        if self._is_linux:
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

        direct_row = ctk.CTkFrame(hid, fg_color="transparent")
        direct_row.pack(fill="x", padx=10, pady=4)
        ctk.CTkLabel(direct_row, text="Direct Mode", width=230, anchor="w").pack(side="left")
        self._direct_var = ctk.BooleanVar(value=self._device_state.get("direct_mode", False))
        ctk.CTkSwitch(direct_row, text="", variable=self._direct_var,
                      command=lambda: self._set_direct_mode(self._direct_var.get())).pack(side="left", padx=4)

        spdif_row = ctk.CTkFrame(hid, fg_color="transparent")
        spdif_row.pack(fill="x", padx=10, pady=4)
        ctk.CTkLabel(spdif_row, text="SPDIF Out Direct Mode", width=230, anchor="w").pack(side="left")
        self._spdif_direct_var = ctk.BooleanVar(value=self._device_state.get("spdif_direct", False))
        ctk.CTkSwitch(spdif_row, text="", variable=self._spdif_direct_var,
                      command=lambda: self._set_spdif_direct(self._spdif_direct_var.get())).pack(side="left", padx=4)

        filter_row = ctk.CTkFrame(hid, fg_color="transparent")
        filter_row.pack(fill="x", padx=10, pady=4)
        ctk.CTkLabel(filter_row, text="DAC Filter", width=230, anchor="w").pack(side="left")
        self._filter_var = ctk.StringVar(value=self._device_state.get("dac_filter", "Fast Roll-off / Minimum Phase"))
        ctk.CTkOptionMenu(
            filter_row, width=320, variable=self._filter_var,
            values=[
                "Fast Roll-off / Minimum Phase",
                "Slow Roll-off / Minimum Phase",
                "Fast Roll-off / Linear Phase",
                "Slow Roll-off / Linear Phase",
            ],
            command=self._set_filter
        ).pack(side="left", padx=4)

        dec_row = ctk.CTkFrame(hid, fg_color="transparent")
        dec_row.pack(fill="x", padx=10, pady=4)
        ctk.CTkLabel(dec_row, text="Decoder Mode", width=230, anchor="w").pack(side="left")
        self._decoder_var = ctk.StringVar(value=self._device_state.get("decoder_mode", "Normal"))
        ctk.CTkOptionMenu(dec_row, values=["Normal", "Full", "Night"], variable=self._decoder_var,
                          width=220, command=self._set_decoder_mode).pack(side="left", padx=4)

        self._audio_banner(sf)

        vol = self._section(sf, "Volume & Mute", badge=self._audio_badge())
        vol.pack(fill="x", padx=4, pady=4)
        self._toggle(vol, "Mute Playback", "playback-mute", claim=True)
        self._slider(vol, "Volume (0–100)", "playback-volume", claim=True)
        self._channels(vol, "Volume Channels", "playback-volume-channels", claim=True)

        note = self._section(sf, "Speaker Layout", badge=self._audio_badge())
        note.pack(fill="x", padx=4, pady=4)
        ctk.CTkLabel(
            note,
            text=("Speaker layout readback is hidden for now. The vendor protocol did not expose a trustworthy live value, "
                  "and Windows/Linux system-level reads are different enough that showing a guessed status was misleading."),
            text_color=("gray45", "gray70"), justify="left", wraplength=640,
            font=ctk.CTkFont(size=11)
        ).pack(anchor="w", padx=10, pady=(0, 8))

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

        mic = self._section(sf, "Microphone Recording", badge=self._audio_badge())
        mic.pack(fill="x", padx=4, pady=4)
        self._toggle(mic, "Mute",       "recording-mute",                         claim=True)
        self._slider(mic, "Volume",     "recording-mic-recording-volume",          lo=0, hi=100, step=10, claim=True)
        self._channels(mic, "Channels", "recording-mic-recording-volume-channels", claim=True)

        mon = self._section(sf, "Microphone Monitoring", badge=self._audio_badge())
        mon.pack(fill="x", padx=4, pady=4)
        self._toggle(mon, "Mute",       "recording-mic-monitoring-mute",           claim=True)
        self._slider(mon, "Volume",     "recording-mic-monitoring-volume",          lo=0, hi=100, step=10, claim=True)
        self._channels(mon, "Channels", "recording-mic-monitoring-volume-channels", claim=True)

    def _build_tab_mixer(self, parent):
        sf = ctk.CTkScrollableFrame(parent)
        sf.pack(fill="both", expand=True)
        self._audio_banner(sf)

        pb = self._section(sf, "Playback Mix", badge=self._audio_badge())
        pb.pack(fill="x", padx=4, pady=4)
        self._toggle(pb, "Mute Playback Mix", "mixer-playback-mute", claim=True)

        mon = self._section(sf, "Monitoring", badge=self._audio_badge())
        mon.pack(fill="x", padx=4, pady=4)
        for label, key in [("Line In", "line-in"), ("External Mic", "external-mic"),
                            ("SPDIF In", "spdif-in")]:
            self._mixer_source(mon, label, f"mixer-monitoring-{key}")

        rec = self._section(sf, "Recording Sources", badge=self._audio_badge())
        rec.pack(fill="x", padx=4, pady=4)
        for label, key in [("Line In", "line-in"), ("External Mic", "external-mic"),
                            ("SPDIF In", "spdif-in"), ("What U Hear", "what-u-hear")]:
            self._mixer_source(rec, label, f"mixer-recording-{key}")


    def _build_tab_lighting(self, parent):
        sf = ctk.CTkScrollableFrame(parent)
        sf.pack(fill="both", expand=True)

        s = self._section(sf, "LED Lighting", badge="[HID]")
        s.pack(fill="x", padx=4, pady=4)

        rgb_row = ctk.CTkFrame(s, fg_color="transparent")
        rgb_row.pack(fill="x", padx=10, pady=(2, 8))
        ctk.CTkLabel(rgb_row, text="RGB Accent Light", width=230, anchor="w").pack(side="left")
        self._rgb_var = ctk.BooleanVar(value=self._device_state.get("rgb_enabled", True))
        ctk.CTkSwitch(rgb_row, text="", variable=self._rgb_var,
                      command=lambda: self._set_rgb_enabled(self._rgb_var.get()), width=50).pack(side="left", padx=4)

        prev_row = ctk.CTkFrame(s, fg_color="transparent")
        prev_row.pack(fill="x", padx=10, pady=(4, 6))
        ctk.CTkLabel(prev_row, text="Custom RGB Color:", width=230, anchor="w").pack(side="left")
        self._preview = ctk.CTkFrame(prev_row, width=52, height=34, corner_radius=7, fg_color="#ff0000")
        self._preview.pack(side="left", padx=8)
        ctk.CTkButton(prev_row, text="Apply Color", width=100, command=self._apply_rgb).pack(side="left", padx=4)

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

        ctk.CTkFrame(s, height=1, fg_color=("gray65", "gray38")).pack(fill="x", padx=10, pady=(10, 5))

        ring_row = ctk.CTkFrame(s, fg_color="transparent")
        ring_row.pack(fill="x", padx=10, pady=(2, 8))
        ctk.CTkLabel(ring_row, text="Volume Ring Light", width=230, anchor="w").pack(side="left")
        self._ring_var = ctk.BooleanVar(value=self._device_state.get("ring_enabled", True))
        ctk.CTkSwitch(ring_row, text="", variable=self._ring_var,
                      command=lambda: self._set_ring_enabled(self._ring_var.get()), width=50).pack(side="left", padx=4)

    def _build_tab_system(self, parent):
        sf = ctk.CTkScrollableFrame(parent)
        sf.pack(fill="both", expand=True)

        info = self._section(sf, "Platform")
        info.pack(fill="x", padx=4, pady=4)
        ctk.CTkLabel(
            info,
            anchor="w",
            justify="left",
            font=ctk.CTkFont(size=11),
            text=(
                f"Detected platform: {self._platform}\n"
                + (
                    "Linux-specific USB claim and audio-driver recovery controls are available below."
                    if self._is_linux
                    else "Linux-specific claim/release and modprobe recovery controls are hidden on this platform."
                )
            ),
        ).pack(padx=10, pady=(2, 8), anchor="w")

        util = self._section(sf, "Utilities")
        util.pack(fill="x", padx=4, pady=4)
        ctk.CTkButton(util, text="Refresh Device Status", anchor="w",
                      command=self._refresh_device_state_async).pack(fill="x", padx=10, pady=2)
        ctk.CTkButton(util, text="Clear Console", anchor="w",
                      command=self._clear_console).pack(fill="x", padx=10, pady=2)

        if self._is_linux:
            rec = self._section(sf, "Linux Audio Recovery")
            rec.pack(fill="x", padx=4, pady=4)
            ctk.CTkLabel(rec, anchor="w", justify="left", font=ctk.CTkFont(size=11),
                         text=(
                             "With 'modprobe (recommended)', the full sequence runs automatically:\n"
                             "stop PipeWire → modprobe -r → command → modprobe → start PipeWire\n\n"
                             "PipeWire must be stopped first or modprobe -r will fail with\n"
                             "'Module snd_usb_audio is in use'.\n\n"
                             "If recovery is needed, run the steps below manually in order:"
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
                ctk.CTkButton(rec, text=label, anchor="w", command=fn).pack(fill="x", padx=10, pady=2)

            claim = self._section(sf, "Linux USB Claim")
            claim.pack(fill="x", padx=4, pady=4)
            ctk.CTkLabel(claim, anchor="w", justify="left", font=ctk.CTkFont(size=11),
                         text="Standalone claim/release without any audio command. Useful only when testing the Linux USB handoff path."
                         ).pack(padx=10, pady=(2, 6), anchor="w")
            ctk.CTkButton(claim, text="Claim & Release",
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
        self._run_async("--lighting-rgb", self._r.get(), self._g.get(), self._b.get(),
                        on_success=lambda: self._state_set(rgb_enabled=True))


if __name__ == "__main__":
    app = G6GUI()
    app.mainloop()
