##: Building the pimonitor executable
- Run `cargo build`.  If any warnings or errors are generated, attempt to fix them and rebuild.

##: Unit testing
- When new functions are created, write a unit test for them.
- Run `cargo test`.  If any tests fail, fix them and rebuild.
- All tests should be created in the `tests` directory.

##: AI/LLM coding
- When AI/LLM coding agents make changes, write a new datestamped file to the `.llm_history` directory outlining the changes made.

##: The pimonitor.yaml configuration file
- This is a YAML configuration file.
- If it does not exist upon startup, it will be created with blank values.
- It should be read right before each polling interval starts.
- It contains the following keys: `pi_api_key`, `pi_api_secret`, `poll_interval`
- The `poll_interval` key is optional. If not provided, the default polling interval is 60 seconds.  If this key is provided, it must be an integer greater than 30 seconds.  If it is not, the default polling interval will be used.  It's value should be stored in a global variable called `POLL_INTERVAL`.
- The `pi_api_key` and `pi_api_secret` hold the API key and secret for read/write operations on the Podcast Index API.
- If either the `pi_api_key` or `pi_api_secret` is missing a global variable should be set called `PI_READ_WRITE` with a value of `false`.
- If both the `pi_api_key` and `pi_api_secret` are present, the `PI_READ_WRITE` variable should be set to `true`.