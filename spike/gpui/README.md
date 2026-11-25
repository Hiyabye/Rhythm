# GPUI feasibility spike (#340)

Throwaway prototype for the v3.0.0 go/no-go. Never merged into `main`; it is its own Cargo workspace.

Stack: `gpui-kit` 0.11.0 (Longbridge's facade over `gpui-component` 0.11.0 and the `gpui-pre` 0.3.14 snapshot of Zed `f126950`, all Apache-2.0 on crates.io), plus tokio, reqwest 0.12, rusqlite 0.38 and `tray-icon` 0.25 (Tauri's tray crate).

```sh
cd spike/gpui
cargo run            # debug
cargo build --release
```

Windows 11 x64 needs the MSVC toolchain from `docs/WINDOWS.md`; no other setup.

## Manual checks

Type with the system Korean IME (macOS 2-Set Korean, Windows Microsoft IME):

1. Single-line field: type `한글 입력.` and press Enter. The log line `input enter` must read `"한글 입력."`. Check that the last syllable isn't dropped when space, `.` or Enter ends it.
2. Type `한`, press Backspace while composing. It must become `하`, not delete the whole syllable.
3. Textarea: `안녕`, Enter, `하세요`, then click outside. The value must be `"안녕\n하세요"`.
4. The candidate/composition window appears at the caret (Hanja: type `한`, press Option+Return on macOS or 한자 key on Windows).
5. Reader: drag-select text and copy; click the link (opens the browser); the remote image stays blank.
6. 비동기: 요청 시작 → status completes in ~2 s while `UI 틱` keeps counting; 취소 mid-request cancels.
7. 미니 창 열기 → small window stays above other apps.
8. Close the main window → the app stays in the tray; tray menu 열기 reopens it; 종료 quits.
9. Switch OS dark/light mode → content follows.
10. Screen reader (VoiceOver / Narrator): fields, buttons and the notice body are announced.
