"""
CustomTkinter window for the SoundBlaster X G6.

- Device changes are executed by the CLI in a subprocess (see runner.py), so a hung USB call never freezes the UI.
- Device state is read in-process over HID (g6_cli.g6_status), which needs no claim and is read-only.
"""
from __future__ import annotations

import platform
import queue
import subprocess
import threading
from datetime import datetime
from typing import Optional

import customtkinter as ctk

from g6_cli.g6_status import DAC_FILTER_LABELS, DeviceStatus, read_status
from g6_cli.gui import controls as C
from g6_cli.gui.runner import CommandRunner, RunnerOptions

WARN_BG = ('#fff3cd', '#3d3000')
WARN_FG = ('#7a5500', '#ffcc44')
OK_BG = ('#dff0d8', '#1a3a1a')
OK_FG = ('#2d6a2d', '#66cc66')
CON_BG = '#0d0d0d'
CON_COLS = {
    'cmd': '#4fc3f7',
    'ok': '#81c784',
    'err': '#ef5350',
    'warn': '#ffb74d',
    'dim': '#616161',
}
LABEL_W = 230


def _on_off(value: Optional[bool]) -> str:
    return '?' if value is None else ('On' if value else 'Off')


# ── Password dialog ───────────────────────────────────────────────────────────

class PasswordDialog(ctk.CTkToplevel):
    def __init__(self, parent, prompt: str = 'Enter sudo password:'):
        super().__init__(parent)
        self.title('Authentication Required')
        self.geometry('360x160')
        self.resizable(False, False)
        self.grab_set()
        self.lift()
        self.focus_force()
        self._result: Optional[str] = None

        ctk.CTkLabel(self, text='🔒  ' + prompt, font=ctk.CTkFont(size=13)).pack(pady=(20, 8))
        self._entry = ctk.CTkEntry(self, show='●', width=260, placeholder_text='password')
        self._entry.pack()
        self._entry.focus_set()
        self._entry.bind('<Return>', lambda e: self._ok())

        row = ctk.CTkFrame(self, fg_color='transparent')
        row.pack(pady=14)
        ctk.CTkButton(row, text='OK', width=100, command=self._ok).pack(side='left', padx=6)
        ctk.CTkButton(row, text='Cancel', width=100, fg_color='transparent', border_width=1,
                      command=self._cancel).pack(side='left', padx=6)
        self.protocol('WM_DELETE_WINDOW', self._cancel)

    def _ok(self):
        self._result = self._entry.get()
        self.destroy()

    def _cancel(self):
        self._result = None
        self.destroy()

    @classmethod
    def ask(cls, parent, prompt: str = 'Enter sudo password:') -> Optional[str]:
        dlg = cls(parent, prompt)
        parent.wait_window(dlg)
        return dlg._result


# ── Main window ───────────────────────────────────────────────────────────────

class G6GUI(ctk.CTk):
    TABS = ('Playback', 'SBX Effects', 'Recording', 'Mixer', 'Lighting', 'System')

    def __init__(self):
        super().__init__()
        self.title('SoundBlaster X G6')
        self.geometry('780x820')
        self.minsize(660, 580)

        self._platform = platform.system()
        self._is_linux = self._platform == 'Linux'
        self._log_queue: queue.Queue = queue.Queue()
        self._ui_queue: queue.Queue = queue.Queue()  # callables from worker threads, run on the Tk thread
        self._status = DeviceStatus()
        self._built_tabs: set[str] = set()
        self._dmesg_proc: Optional[subprocess.Popen] = None

        self._dry_run = ctk.BooleanVar(value=False)
        self._auto_reload = ctk.BooleanVar(value=True)
        self._use_modprobe = ctk.BooleanVar(value=True)
        self._use_sudo = ctk.BooleanVar(value=True)
        self._settle_ms = ctk.IntVar(value=600)
        self._show_dmesg = ctk.BooleanVar(value=False)

        # state-bound widgets' variables (created when their tab is built)
        self._vars: dict[str, ctk.Variable] = {}

        self._runner = CommandRunner(log=self._log, ask_password=self._ask_password_threadsafe,
                                     is_linux=self._is_linux)

        self._build_chrome()
        self.after(0, lambda: self._ensure_tab('Playback'))
        self._drain_log_queue()
        self._refresh_status_async()

    # ── Running commands ──────────────────────────────────────────────────────

    def _runner_options(self) -> RunnerOptions:
        return RunnerOptions(dry_run=self._dry_run.get(), auto_reload=self._auto_reload.get(),
                             use_modprobe=self._use_modprobe.get(), use_sudo=self._use_sudo.get(),
                             settle_ms=max(0, int(self._settle_ms.get())))

    def run(self, *args, claim: bool = False, on_success=None):
        """Run a single CLI invocation. ``on_success`` runs on the UI thread."""
        self.run_steps([list(map(str, args))], claim=claim, on_success=on_success)

    def run_steps(self, steps: list[list[str]], claim: bool = False, on_success=None):
        if self._dry_run.get():
            on_success = None  # nothing changed on the device, so don't pretend it did
        callback = (lambda: self.call_ui(on_success)) if on_success else None
        self._runner.run_async(steps, self._runner_options(), claim=claim, on_success=callback)

    def _ask_password_threadsafe(self) -> Optional[str]:
        """Called from worker threads: show the dialog on the Tk thread and wait for the answer."""
        if threading.current_thread() is threading.main_thread():
            return PasswordDialog.ask(self)
        done = threading.Event()
        result: list[Optional[str]] = [None]

        def ask():
            result[0] = PasswordDialog.ask(self)
            done.set()

        self.call_ui(ask)
        done.wait()
        return result[0]

    # ── Device state ──────────────────────────────────────────────────────────

    def _refresh_status_async(self):
        def work():
            try:
                status = read_status()
            except ImportError:
                self._log('hid module not found — device state unavailable', 'dim')
                return
            except (IOError, OSError) as e:
                self._log(f'Status read failed: {e}', 'err')
                return
            self.call_ui(lambda: self._apply_status(status))

        threading.Thread(target=work, daemon=True).start()

    def _apply_status(self, status: DeviceStatus):
        self._status = status
        if status.firmware:
            self.title(f'SoundBlaster X G6  —  fw {status.firmware}')
        self._sync_vars()
        flags = (f'  flags=0x{status.mode_flags[0]:02x}/0x{status.mode_flags[1]:02x}'
                 if status.mode_flags else '')
        self._log(
            f'Device  output={status.output or "?"}  decoder={status.decoder_mode or "?"}  '
            f'filter={status.dac_filter or "?"}  direct={_on_off(status.direct_mode)}  '
            f'spdif-direct={_on_off(status.spdif_direct)}  rgb={_on_off(status.rgb_enabled)}  '
            f'ring={_on_off(status.ring_enabled)}{flags}', 'dim')

    def _set_state(self, **changes):
        for k, v in changes.items():
            setattr(self._status, k, v)
        self._sync_vars()

    def _sync_vars(self):
        s = self._status
        bound = {
            'decoder': s.decoder_mode if s.decoder_mode in C.DECODER_MODES else None,
            'filter': DAC_FILTER_LABELS.get(s.dac_filter),
            'direct': s.direct_mode,
            'spdif_direct': s.spdif_direct,
            'rgb': s.rgb_enabled,
            'ring': s.ring_enabled,
        }
        for key, value in bound.items():
            if key in self._vars and value is not None:
                self._vars[key].set(value)

        cards = {
            'output': s.output or '?',
            'decoder': s.decoder_mode or '?',
            'filter': DAC_FILTER_LABELS.get(s.dac_filter, s.dac_filter or '?'),
            'direct': _on_off(s.direct_mode),
            'spdif': _on_off(s.spdif_direct),
            'rgb': _on_off(s.rgb_enabled),
            'ring': _on_off(s.ring_enabled),
            'fw': s.firmware or '?',
        }
        for key, value in cards.items():
            self._status_vars[key].set(value)

    # ── Actions with state side effects ───────────────────────────────────────

    def _set_output(self, value: str):
        self.run('--set-output', value, on_success=lambda: self._set_state(output=value))

    def _set_direct_mode(self, enabled: bool):
        # Direct mode and SPDIF-out direct mode are mutually exclusive.
        steps = []
        if enabled and self._status.spdif_direct:
            steps.append(['--playback-spdif-out-direct-mode', 'Disabled'])
        steps.append(['--playback-direct-mode', 'Enabled' if enabled else 'Disabled'])
        changes = {'direct_mode': enabled, **({'spdif_direct': False} if enabled else {})}
        self.run_steps(steps, on_success=lambda: self._set_state(**changes))

    def _set_spdif_direct(self, enabled: bool):
        steps = []
        if enabled and self._status.direct_mode:
            steps.append(['--playback-direct-mode', 'Disabled'])
        steps.append(['--playback-spdif-out-direct-mode', 'Enabled' if enabled else 'Disabled'])
        changes = {'spdif_direct': enabled, **({'direct_mode': False} if enabled else {})}
        self.run_steps(steps, on_success=lambda: self._set_state(**changes))

    def _set_decoder_mode(self, value: str):
        self.run('--decoder-mode', value, on_success=lambda: self._set_state(decoder_mode=value))

    def _set_filter(self, label: str):
        value = C.DAC_FILTERS[label]
        self.run('--playback-filter', value, on_success=lambda: self._set_state(dac_filter=value))

    def _set_layout(self, output: str, layout: str):
        self.run(C.SPEAKER_LAYOUT_FLAGS[(output, layout)], claim=True)

    def _set_rgb_enabled(self, enabled: bool):
        if enabled:
            self._apply_rgb()
        else:
            self.run('--lighting-disable', on_success=lambda: self._set_state(rgb_enabled=False))

    def _apply_rgb(self):
        self._update_preview()
        self.run('--lighting-rgb', self._r.get(), self._g.get(), self._b.get(),
                 on_success=lambda: self._set_state(rgb_enabled=True))

    def _set_ring_enabled(self, enabled: bool):
        self.run('--lighting-volume-ring', 'Enabled' if enabled else 'Disabled',
                 on_success=lambda: self._set_state(ring_enabled=enabled))

    # ── Console ───────────────────────────────────────────────────────────────

    def call_ui(self, fn) -> None:
        """Thread-safe: run ``fn`` on the Tk thread (Tk itself must only be touched from there)."""
        self._ui_queue.put(fn)

    def _log(self, text: str, kind: str = 'cmd'):
        ts = datetime.now().strftime('%H:%M:%S')
        self._log_queue.put((f'[{ts}] {text}\n', kind))

    def _drain_log_queue(self):
        try:
            while True:
                text, kind = self._log_queue.get_nowait()
                self._console.configure(state='normal')
                tw = self._console._textbox
                start = tw.index('end-1c')
                self._console.insert('end', text)
                tag = f'col_{kind}'
                if tag not in tw.tag_names():
                    tw.tag_configure(tag, foreground=CON_COLS.get(kind, '#ffffff'))
                tw.tag_add(tag, start, 'end-1c')
                self._console.configure(state='disabled')
                self._console.see('end')
        except queue.Empty:
            pass
        while True:
            try:
                fn = self._ui_queue.get_nowait()
            except queue.Empty:
                break
            try:
                fn()
            except Exception as e:  # keep the UI loop alive
                self._log(f'UI callback failed: {e}', 'err')
        self.after(80, self._drain_log_queue)

    def _clear_console(self):
        self._console.configure(state='normal')
        self._console.delete('1.0', 'end')
        self._console.configure(state='disabled')

    # ── dmesg tail (Linux) ────────────────────────────────────────────────────

    def _toggle_dmesg(self, *_):
        if self._show_dmesg.get():
            if self._dmesg_proc is None:
                try:
                    self._dmesg_proc = subprocess.Popen(['dmesg', '-w', '--level=err,warn,info'],
                                                        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                                        text=True, bufsize=1)
                except FileNotFoundError:
                    self._log('dmesg not available', 'dim')
                    self._show_dmesg.set(False)
                    return
                threading.Thread(target=self._read_dmesg, args=(self._dmesg_proc,), daemon=True).start()
                self._log('dmesg tail started', 'dim')
        elif self._dmesg_proc:
            self._dmesg_proc.kill()
            self._dmesg_proc = None
            self._log('dmesg tail stopped', 'dim')

    def _read_dmesg(self, proc: subprocess.Popen):
        try:
            for line in proc.stdout:
                if line.strip():
                    self._log(line.strip(), 'dim')
        except (OSError, ValueError):
            pass

    # ── Widget helpers ────────────────────────────────────────────────────────

    def _audio_badge(self) -> str:
        return '[Audio · Linux]' if self._is_linux else '[Audio]'

    def _section(self, parent, title: str, badge: str = '') -> ctk.CTkFrame:
        frame = ctk.CTkFrame(parent)
        frame.pack(fill='x', padx=4, pady=4)
        hdr = ctk.CTkFrame(frame, fg_color='transparent')
        hdr.pack(fill='x', padx=10, pady=(8, 0))
        ctk.CTkLabel(hdr, text=title, font=ctk.CTkFont(size=13, weight='bold'), anchor='w').pack(side='left')
        if badge:
            audio = badge.startswith('[Audio')
            ctk.CTkLabel(hdr, text=badge, font=ctk.CTkFont(size=10, weight='bold'),
                         text_color=WARN_FG if audio else OK_FG, fg_color=WARN_BG if audio else OK_BG,
                         corner_radius=4).pack(side='left', padx=6)
        ctk.CTkFrame(frame, height=1, fg_color=('gray65', 'gray38')).pack(fill='x', padx=10, pady=(3, 5))
        return frame

    def _audio_banner(self, parent):
        f = ctk.CTkFrame(parent, fg_color=WARN_BG, corner_radius=6)
        f.pack(fill='x', padx=6, pady=(6, 2))
        if self._is_linux:
            text = ('⚠  Linux audio commands need an exclusive USB claim.\n'
                    '   PipeWire stopped → snd_usb_audio removed → command → module + PipeWire restored.')
        else:
            text = ('ℹ  Driver recovery controls are Linux-only.\n'
                    '   On Windows, playback/recording commands run without the Linux claim/release flow.')
        ctk.CTkLabel(f, text=text, text_color=WARN_FG, font=ctk.CTkFont(size=11), justify='left',
                     anchor='w').pack(padx=10, pady=5, anchor='w')

    def _row(self, parent, label: str, padx: int = 10) -> ctk.CTkFrame:
        row = ctk.CTkFrame(parent, fg_color='transparent')
        row.pack(fill='x', padx=padx, pady=2)
        ctk.CTkLabel(row, text=label, width=LABEL_W, anchor='w').pack(side='left')
        return row

    def _switch(self, parent, label: str, key: str, default: bool, command):
        row = self._row(parent, label)
        var = ctk.BooleanVar(value=default)
        self._vars[key] = var
        ctk.CTkSwitch(row, text='', variable=var, width=50, command=lambda: command(var.get())).pack(side='left',
                                                                                                  padx=4)
        return var

    def _control(self, parent, control, claim: bool, padx: int = 10):
        """Build one widget from a controls.* description."""
        row = self._row(parent, control.label, padx=padx)
        if isinstance(control, C.Toggle):
            for value in C.ENABLED_DISABLED:
                ctk.CTkButton(row, text=value, width=95, fg_color='transparent', border_width=1,
                              command=lambda v=value: self.run(*C.command_for(control, v), claim=claim)
                              ).pack(side='left', padx=2)
        elif isinstance(control, C.Option):
            var = ctk.StringVar(value=control.values[0])
            ctk.CTkOptionMenu(row, values=list(control.values), variable=var, width=270,
                              command=lambda v: self.run(*C.command_for(control, v), claim=claim)
                              ).pack(side='left', padx=4)
        elif isinstance(control, C.Slider):
            var = ctk.IntVar(value=control.default)
            channels = ctk.StringVar(value='Both')
            ctk.CTkLabel(row, textvariable=var, width=36, anchor='e').pack(side='right')
            if control.channels_flag:
                ctk.CTkSegmentedButton(row, values=list(C.CHANNELS), variable=channels, width=150
                                       ).pack(side='right', padx=4)
            slider = ctk.CTkSlider(row, from_=control.lo, to=control.hi, variable=var,
                                   number_of_steps=max(1, (control.hi - control.lo) // control.step))
            slider.pack(side='left', fill='x', expand=True, padx=4)
            slider.bind('<ButtonRelease-1>', lambda e: self.run(
                *C.command_for(control, int(var.get()), channels.get()), claim=claim))
        else:  # pragma: no cover
            raise TypeError(control)

    def _table_section(self, parent, section: C.Section):
        frame = self._section(parent, section.title, badge=self._audio_badge() if section.audio else '[HID]')
        for control in section.controls:
            self._control(frame, control, claim=section.audio)

    def _mixer_group(self, parent, group: C.MixerGroup):
        frame = self._section(parent, group.title, badge=self._audio_badge())
        for label, ctrls in group.sources:
            ctk.CTkLabel(frame, text=label, font=ctk.CTkFont(size=12, weight='bold'), anchor='w'
                         ).pack(anchor='w', padx=14, pady=(7, 0))
            for control in ctrls:
                self._control(frame, control, claim=True, padx=24)

    # ── Chrome ────────────────────────────────────────────────────────────────

    def _build_chrome(self):
        hdr = ctk.CTkFrame(self, height=54, corner_radius=0)
        hdr.pack(fill='x')
        hdr.pack_propagate(False)
        ctk.CTkLabel(hdr, text='SoundBlaster X G6', font=ctk.CTkFont(size=17, weight='bold')).pack(side='left',
                                                                                                   padx=16)
        ctk.CTkLabel(hdr, text='Output:').pack(side='left', padx=(20, 4))
        for val in C.OUTPUTS:
            ctk.CTkButton(hdr, text=val, width=95, fg_color='transparent', border_width=1,
                          command=lambda v=val: self._set_output(v)).pack(side='left', padx=2)
        ctk.CTkButton(hdr, text='Toggle', width=72, command=self._toggle_output).pack(side='left', padx=7)
        ctk.CTkCheckBox(hdr, text='Dry Run', variable=self._dry_run, width=95).pack(side='right', padx=14)

        self._build_platform_bar()

        status = ctk.CTkFrame(self, fg_color=('gray90', 'gray16'), corner_radius=0)
        status.pack(fill='x')
        cards = [('Output', 'output'), ('Decoder', 'decoder'), ('Filter', 'filter'), ('Direct', 'direct'),
                 ('SPDIF Direct', 'spdif'), ('RGB', 'rgb'), ('Ring', 'ring'), ('Firmware', 'fw')]
        self._status_vars = {key: ctk.StringVar(value='?') for _, key in cards}
        for title, key in cards:
            card = ctk.CTkFrame(status, fg_color='transparent')
            card.pack(side='left', padx=8, pady=4)
            ctk.CTkLabel(card, text=title, font=ctk.CTkFont(size=10), text_color=('gray45', 'gray70')
                         ).pack(anchor='w')
            ctk.CTkLabel(card, textvariable=self._status_vars[key], font=ctk.CTkFont(size=12, weight='bold')
                         ).pack(anchor='w')
        ctk.CTkButton(status, text='Refresh', width=72, height=26, command=self._refresh_status_async
                      ).pack(side='right', padx=10, pady=6)

        main = ctk.CTkFrame(self, fg_color='transparent')
        main.pack(fill='both', expand=True)
        main.rowconfigure(0, weight=3)
        main.rowconfigure(2, weight=1, minsize=120)
        main.columnconfigure(0, weight=1)

        self._tabs = ctk.CTkTabview(main, command=lambda: self._ensure_tab(self._tabs.get()))
        self._tabs.grid(row=0, column=0, sticky='nsew', padx=10, pady=(6, 0))
        for name in self.TABS:
            self._tabs.add(name)

        div = ctk.CTkFrame(main, height=28, fg_color=('gray85', 'gray20'), corner_radius=0)
        div.grid(row=1, column=0, sticky='ew')
        ctk.CTkLabel(div, text='  OUTPUT', anchor='w', font=ctk.CTkFont(size=10, weight='bold'),
                     text_color=('gray50', 'gray55')).pack(side='left', padx=8)
        ctk.CTkButton(div, text='Clear', width=55, height=20, fg_color='transparent', border_width=1,
                      font=ctk.CTkFont(size=10), command=self._clear_console).pack(side='right', padx=8, pady=4)
        if self._is_linux:
            ctk.CTkCheckBox(div, text='dmesg', variable=self._show_dmesg, font=ctk.CTkFont(size=10),
                            command=self._toggle_dmesg).pack(side='right', padx=4)

        self._console = ctk.CTkTextbox(main, state='disabled', fg_color=CON_BG, text_color='#cccccc',
                                       font=ctk.CTkFont(family='monospace', size=11), corner_radius=0, wrap='word')
        self._console.grid(row=2, column=0, sticky='nsew')

    def _build_platform_bar(self):
        cb = ctk.CTkFrame(self, height=38, corner_radius=0, fg_color=WARN_BG)
        cb.pack(fill='x')
        cb.pack_propagate(False)
        small = ctk.CTkFont(size=11)
        if not self._is_linux:
            ctk.CTkLabel(cb, text=f'{self._platform} mode: Linux claim/release and modprobe recovery are hidden.',
                         text_color=WARN_FG, font=small).pack(side='left', padx=10)
            return
        ctk.CTkLabel(cb, text='[Audio · Linux]:', text_color=WARN_FG,
                     font=ctk.CTkFont(size=11, weight='bold')).pack(side='left', padx=10)
        for text, var in [('Auto-restore audio', self._auto_reload), ('modprobe (recommended)', self._use_modprobe),
                          ('sudo', self._use_sudo)]:
            ctk.CTkCheckBox(cb, text=text, variable=var, text_color=WARN_FG, font=small).pack(side='left', padx=6)
        ctk.CTkLabel(cb, text='Settle ms:', text_color=WARN_FG, font=small).pack(side='left', padx=(12, 2))
        ctk.CTkSlider(cb, from_=0, to=2000, variable=self._settle_ms, number_of_steps=40, width=100
                      ).pack(side='left')
        ctk.CTkLabel(cb, textvariable=self._settle_ms, width=36, text_color=WARN_FG, font=small
                     ).pack(side='left', padx=2)
        ctk.CTkButton(cb, text='Clear password', width=110, fg_color='transparent', border_width=1,
                      text_color=WARN_FG, border_color=WARN_FG, font=small, command=self._clear_password
                      ).pack(side='right', padx=10)

    def _clear_password(self):
        self._runner.clear_password()
        self._log('Sudo password cleared', 'warn')

    def _toggle_output(self):
        # Use the read-back output when we have it; the CLI's --toggle-output only remembers its own last call.
        current = self._status.output
        if current in C.OUTPUTS:
            self._set_output('Headphones' if current == 'Speakers' else 'Speakers')
        else:
            self.run('--toggle-output', on_success=self._refresh_status_async)

    def _ensure_tab(self, name: str):
        if name in self._built_tabs:
            return
        self._built_tabs.add(name)
        sf = ctk.CTkScrollableFrame(self._tabs.tab(name))
        sf.pack(fill='both', expand=True)
        getattr(self, '_build_tab_' + name.split()[0].lower())(sf)
        self._sync_vars()

    # ── Tabs ──────────────────────────────────────────────────────────────────

    def _build_tab_playback(self, sf):
        hid = self._section(sf, 'Mode & Filter', badge='[HID]')
        self._switch(hid, 'Direct Mode', 'direct', bool(self._status.direct_mode), self._set_direct_mode)
        self._switch(hid, 'SPDIF Out Direct Mode', 'spdif_direct', bool(self._status.spdif_direct),
                     self._set_spdif_direct)

        row = self._row(hid, 'DAC Filter')
        self._vars['filter'] = ctk.StringVar(value=next(iter(C.DAC_FILTERS)))
        ctk.CTkOptionMenu(row, width=320, variable=self._vars['filter'], values=list(C.DAC_FILTERS),
                          command=self._set_filter).pack(side='left', padx=4)

        row = self._row(hid, 'Decoder Mode')
        self._vars['decoder'] = ctk.StringVar(value='Normal')
        ctk.CTkOptionMenu(row, values=list(C.DECODER_MODES), variable=self._vars['decoder'], width=220,
                          command=self._set_decoder_mode).pack(side='left', padx=4)

        self._audio_banner(sf)
        self._table_section(sf, C.PLAYBACK_VOLUME)
        self._table_section(sf, C.LINE_OUT)

        layout = self._section(sf, 'Speaker Layout', badge=self._audio_badge())
        for output in C.OUTPUTS:
            row = self._row(layout, output)
            for name in ('Stereo', '5.1', '7.1'):
                ctk.CTkButton(row, text=name, width=75, fg_color='transparent', border_width=1,
                              command=lambda o=output, n=name: self._set_layout(o, n)).pack(side='left', padx=2)
        ctk.CTkLabel(layout, text='No readback for the layout yet — the current value is not shown.',
                     text_color=('gray45', 'gray70'), font=ctk.CTkFont(size=11)).pack(anchor='w', padx=10,
                                                                                     pady=(2, 8))

    def _build_tab_sbx(self, sf):
        self._table_section(sf, C.SBX_MODE)
        for section in C.SBX_SECTIONS:
            self._table_section(sf, section)
        self._table_section(sf, C.EQUALIZER)

    def _build_tab_recording(self, sf):
        self._table_section(sf, C.VOICE_CLARITY)
        self._audio_banner(sf)
        self._table_section(sf, C.MIC_RECORDING)
        self._table_section(sf, C.MIC_MONITORING)

    def _build_tab_mixer(self, sf):
        self._audio_banner(sf)
        self._table_section(sf, C.MIXER_PLAYBACK)
        self._mixer_group(sf, C.MIXER_MONITORING)
        self._mixer_group(sf, C.MIXER_RECORDING)

    def _build_tab_lighting(self, sf):
        s = self._section(sf, 'LED Lighting', badge='[HID]')
        self._switch(s, 'RGB Accent Light', 'rgb', bool(self._status.rgb_enabled), self._set_rgb_enabled)

        row = self._row(s, 'Custom RGB Color:')
        self._preview = ctk.CTkFrame(row, width=52, height=34, corner_radius=7, fg_color='#ff0000')
        self._preview.pack(side='left', padx=8)
        ctk.CTkButton(row, text='Apply Color', width=100, command=self._apply_rgb).pack(side='left', padx=4)

        self._r, self._g, self._b = ctk.IntVar(value=255), ctk.IntVar(value=0), ctk.IntVar(value=0)
        for ch, var in [('R', self._r), ('G', self._g), ('B', self._b)]:
            r = ctk.CTkFrame(s, fg_color='transparent')
            r.pack(fill='x', padx=10, pady=2)
            ctk.CTkLabel(r, text=ch, width=20, anchor='w').pack(side='left')
            ctk.CTkLabel(r, textvariable=var, width=36, anchor='e').pack(side='right')
            sl = ctk.CTkSlider(r, from_=0, to=255, variable=var, number_of_steps=255,
                               command=lambda _: self._update_preview())
            sl.pack(side='left', fill='x', expand=True, padx=4)
            sl.bind('<ButtonRelease-1>', lambda e: self._apply_rgb())
        self._update_preview()

        ctk.CTkFrame(s, height=1, fg_color=('gray65', 'gray38')).pack(fill='x', padx=10, pady=(10, 5))
        self._switch(s, 'Volume Ring Light', 'ring', bool(self._status.ring_enabled), self._set_ring_enabled)

    def _build_tab_system(self, sf):
        info = self._section(sf, 'Platform')
        ctk.CTkLabel(info, anchor='w', justify='left', font=ctk.CTkFont(size=11), text=(
            f'Detected platform: {self._platform}\n'
            + ('Linux-specific USB claim and audio-driver recovery controls are available below.'
               if self._is_linux else
               'Linux-specific claim/release and modprobe recovery controls are hidden on this platform.')
        )).pack(padx=10, pady=(2, 8), anchor='w')

        util = self._section(sf, 'Utilities')
        ctk.CTkButton(util, text='Refresh Device Status', anchor='w', command=self._refresh_status_async
                      ).pack(fill='x', padx=10, pady=2)
        ctk.CTkButton(util, text='Clear Console', anchor='w', command=self._clear_console
                      ).pack(fill='x', padx=10, pady=2)

        if not self._is_linux:
            return

        rec = self._section(sf, 'Linux Audio Recovery')
        ctk.CTkLabel(rec, anchor='w', justify='left', font=ctk.CTkFont(size=11), text=(
            "With 'modprobe (recommended)', the full sequence runs automatically:\n"
            'stop PipeWire → modprobe -r → command → modprobe → start PipeWire\n\n'
            "PipeWire must be stopped first or modprobe -r fails with 'Module snd_usb_audio is in use'.\n\n"
            'If recovery is needed, run the steps below manually in order:'
        )).pack(padx=10, pady=(2, 8), anchor='w')
        bg = self._runner.run_in_background
        for label, fn in [
            ('1 · systemctl --user stop pipewire pipewire-pulse wireplumber',
             lambda: bg(lambda: self._runner.systemctl_user('stop'))),
            ('2 · sudo modprobe -r snd_usb_audio', lambda: bg(lambda: self._runner.sudo(['modprobe', '-r',
                                                                                          'snd_usb_audio']))),
            ('3 · sudo modprobe snd_usb_audio', lambda: bg(lambda: self._runner.sudo(['modprobe',
                                                                                       'snd_usb_audio']))),
            ('4 · systemctl --user start pipewire pipewire-pulse wireplumber',
             lambda: bg(lambda: self._runner.systemctl_user('start'))),
            ('— or — Reload audio services (sudo)', lambda: self.run('--reload-audio-services')),
        ]:
            ctk.CTkButton(rec, text=label, anchor='w', command=fn).pack(fill='x', padx=10, pady=2)

        claim = self._section(sf, 'Linux USB Claim')
        ctk.CTkLabel(claim, anchor='w', justify='left', font=ctk.CTkFont(size=11),
                     text='Standalone claim/release without any audio command. Only useful for testing the '
                          'Linux USB handoff path.').pack(padx=10, pady=(2, 6), anchor='w')
        ctk.CTkButton(claim, text='Claim & Release', fg_color=WARN_BG, text_color=WARN_FG,
                      hover_color=('#ffe080', '#5a4800'), command=lambda: self.run('--claim-and-release')
                      ).pack(anchor='w', padx=10, pady=(0, 8))

    def _update_preview(self):
        if hasattr(self, '_preview'):
            self._preview.configure(fg_color=f'#{self._r.get():02x}{self._g.get():02x}{self._b.get():02x}')


def main():
    ctk.set_appearance_mode('dark')
    ctk.set_default_color_theme('dark-blue')
    G6GUI().mainloop()
