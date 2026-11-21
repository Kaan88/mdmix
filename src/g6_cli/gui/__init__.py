"""GUI for the SoundBlaster X G6 (needs the optional 'gui' extra: customtkinter)."""


def main() -> None:
    from g6_cli.gui.app import main as _main
    _main()
