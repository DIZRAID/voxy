---
name: Bug report
about: Something in Voxy does not work as expected
title: ""
labels: bug
assignees: ""
---

<!-- English or Russian is fine. -->

**What happened**

<!-- What did you do, what did you expect, and what happened instead? -->

**Steps to reproduce**

1.
2.
3.

**Environment**

- OS and version: <!-- e.g. macOS 15.5 / Windows 11 24H2. Windows: run `winver` to see the version and build -->
- Computer and CPU: <!-- e.g. MacBook Air M2, 2022 / Intel Core i5-12400 -->
- Voxy version: <!-- Shown at the bottom of the Settings sidebar. Built from source: output of `git rev-parse --short HEAD` -->
- Selected model: <!-- Settings → Model, e.g. Parakeet TDT 0.6B v3, or an online provider: OpenAI / Groq / ElevenLabs -->
- Hotkey and mode: <!-- e.g. Right Command or Right Ctrl, Hold / Toggle / Dynamic -->
- macOS permissions granted: <!-- Accessibility / Input Monitoring / Microphone -->
- Windows: does the app you dictate into run as administrator? <!-- yes / no / not sure -->

**Log**

<!--
Paste the lines from around the time of the problem.
  macOS:    tail -n 100 ~/Library/Logs/Voxy.log
  Windows:  Get-Content "$env:LOCALAPPDATA\com.dizraid.voice\Voxy.log" -Tail 100    (PowerShell)
Many log messages are in Russian; paste them unchanged.
Voxy does not write your dictated text to the log, but look through the lines
for anything private (for example, your user name in file paths) before posting.
-->

```text

```
