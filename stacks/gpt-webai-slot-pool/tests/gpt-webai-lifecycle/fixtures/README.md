# gpt-webai-lifecycle fixtures

`fake-bin/gpt-webai-provider` is retained only as a legacy fixture while the
one-shot Rust lifecycle + Node Playwright provider cutover settles.

The historical Bash lifecycle fake suite has been retired. Current offline
coverage lives in the Rust integration tests under
`crates/gpt-webai-lifecycle/tests/...` and Node provider tests under
`provider/chatgpt-playwright/test/...`.
