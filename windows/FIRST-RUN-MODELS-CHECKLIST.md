# First-run models: checklist (Windows)

`Inkwell.Core.Tests` prove what the first run's models step and Settings > Models send and say for
each event: nothing downloads before a Download button, one `model.update` at a time with model and
next the same id, progress, failures with Retry, and the list asked again after each download. What
only a desktop session can show is the real download: the step, the bars, a failure in words, and
the models then doing their jobs. Nothing here was run from an SSH session, and no model was
downloaded to write it.

The downloads are real (Qwen3-ASR 1.7B alone is about 2.3 GB, from huggingface.co). Use a library
of its own so nothing touches the one you use: the models go into `<library>\models`.

## Setup

In a **Developer Command Prompt for VS** (x64), from the repository. The core is built with its
engines, as for S3.5a: without them Silero VAD is not listed, and Qwen3-ASR downloads but cannot
run.

```
cd core
set LIBCLANG_PATH=C:\Program Files\LLVM\bin
set CMAKE_GENERATOR=Ninja
cargo build --release -p ink-ffi --lib --features engine-llama,ink-engines/engine-llama-vulkan,ink-engines/engine-silero
cd ..\windows
dotnet publish Inkwell\Inkwell.csproj -c Release -o %TEMP%\inkwell-app -p:InkCoreDir=%CD%\..\core\target\release\
set INK_DATA_DIR=%TEMP%\inkwell-first-run
%TEMP%\inkwell-app\Inkwell.exe
```

## A. The step, before Download

- [ ] The first run has five dots. After the permissions, a step "Models" lists each model that is
      not installed: its name, licence and size, and "not installed".
- [ ] Under the rows, one line gives the total and where the files come from ("from
      huggingface.co and GitHub": Silero VAD's file is on GitHub), then "Nothing downloads until
      you press Download."
- [ ] Nothing downloads before the button: `%TEMP%\inkwell-first-run\models` stays empty while
      you go Back, Continue and Skip.
- [ ] Narrator reads the step's heading, the rows, and the Download button's name: the models,
      how much in all, and from where.

## B. Download

- [ ] **Download**: the first model reads "Starting the download…", then a bar with "… of …";
      the others read "Waiting for the download before it". The button and its line go, and "You
      can go on: …" shows.
- [ ] **Continue** goes on while it downloads; finish the first run. Settings > Models shows the
      same bars.
- [ ] When a model finishes, its row reads "installed" and the next one starts. In Settings >
      Models, the lines of the jobs it does change to it without leaving the screen (Qwen3-ASR:
      Dictation and Meeting transcript). Once Qwen3-ASR is in, dictation works, and the first
      dictation after it is as quick as the next (it is loaded when its download ends, not by that
      take).

## C. A failure, and Retry

- [ ] Turn the network off during a download: the row reads "Couldn't download …: …" in words,
      with **Retry**; a model waiting behind it starts, and fails the same way while offline.
- [ ] Network on, **Retry**: after "Starting the download…" (the core first re-reads what it has),
      the bar picks up where it stopped, not from zero, and finishes.

## D. Settings > Models

- [ ] Each model not installed has its own **Download**, and "From huggingface.co" (Silero VAD:
      "From GitHub") under its line; an installed one has neither. The heading no longer says
      "Read-only".
- [ ] Two Downloads in a row: the second waits for the first.
- [ ] Quit during a download (notification area > Quit), start again: the model is not installed,
      and **Download** resumes it.

Date, Windows build and commit:
