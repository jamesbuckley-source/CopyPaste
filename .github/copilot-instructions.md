# Working on CopyPaste

## Parallel agents

Run **at most three** delegated agents or sessions at a time. Wait for one to report
before starting the next, unless the user explicitly asks for more. Running more than
that saturates the machine — Rust release builds and Vite builds go from seconds to
minutes, which makes every agent slower and the feedback loop useless.
