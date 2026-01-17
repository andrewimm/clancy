# Claude Code Instructions

All implementation details are covered in `DESIGN.md`. **ALWAYS** consult that file for details on the CLI tool and how it is intended to be used.

All code is written in Rust, in a single crate.

## Tracking Work Progress

Progress is tracked in TRACKING.md. If that file does not exist in the root
directory, create it.

The implementation plan is discussed in DESIGN.md. Each time a feature
is implemented, add a bullet point to TRACKING.md describing the work that
was done. This means TRACKING.md will always have an up-to-date view of
implementation progress.

TRACKING.md should be consulted to know what work has previously been completed.

## Code Quality

1. **Use Rust idioms**
   - Prefer `Option<T>` over nullable values
   - Use `Result<T, E>`  for operations that can fail
   - Leverage pattern matching
   - Use iterator methods where appropriate

2. **Documentation**:
   - Add doc comments (`///`) for public functions and structs
   - Include examples in doc comments *when helpful*
   - Document invariants and safety requirements for unsafe code
   - Comment non-obvious algorithm choices

3. **Error handling**
   - Use `panic!` only for unrecoverable errors
   - Provide clear error messages with context

## Testing Strategy

### Test Requirements

1. Write tests as you implement
3. Run tests frequently:
   ```bash
   cargo test           # All tests
   cargo test test_name # Specific test
   ```

4. One behavior, one test. Each test should cover one expected behavior. Different cases should get their own tests.

## Building and Running

### Development Build
```bash
cargo build
cargo run
```

### Running Tests
```bash
cargo test
cargo test -- --nocapture  # show println! output

## Git Workflow

### Commit Standards

**CRITICAL**: Make incremental, atomic commits that represent single logical
changes. Combine changes with tests where appropriate.

When changes are complete, commit all files with a commit message that explains
what was done, and for what purpose. If this commit unlocks new work items, make
note of that.

#### Good Commit Examples:
```
✓ Access API key from environment variable
✓ Implement notes editing with /notes command
✓ Add lookup cache
```

#### Bad Commit Examples:
```
✗ Implement all of Phase 1
✗ Fix stuff
✗ WIP
✗ Updates
```

### Commit Guidelines

1. **One complete logical change per commit**
   - Adding a single struct and its use case
   - Fixing a specific bug
   - Adding a specific test

2. **Commit message format**:
   ```
   <type>: <concise description>
   
   Optional longer description as needed, to explain why the change was made
   or any important details.
   ```

3. **Commit types**
   - `feat`: New feature or functionality
   - `fix`: Bug fix
   - `refactor`: Code restructuring without behavior changes
   - `test`: Adding or updating tests
   - `docs`: Documentation changes
   - `perf`: Performance improvements
   - `style`: Code style/formatting changes

### Before Each Commit

- Run `cargo check` to ensure code compiles
- Run `cargo test` to make sure tests still pass, even if tests don't cover the
  area that was changed
- Run `cargo fmt` to format code, if any Rust files were modified
- Verify your change is complete and functional

### After Each Commit

- Run `git status` and make sure there are no uncommitted changes. If there are,
add them to the most recent commit or create a new commit as appropriate.
