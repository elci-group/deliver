# Pre-commit Hook Integration

Deliver can be integrated with pre-commit hooks to automatically validate your deliverables before each commit.

## Installation

1. Install pre-commit if you haven't already:
```bash
pip install pre-commit
```

2. Add the hook configuration to your repository by copying the `.pre-commit-config.yaml` file to your project root.

3. Install the hooks:
```bash
pre-commit install
```

## Usage

The pre-commit hook will automatically run `deliver --spec deliver.toml --strict` before each commit. If any validation fails, the commit will be blocked.

### Configuration

Edit `.pre-commit-config.yaml` to customize the hook behavior:

```yaml
repos:
  - repo: local
    hooks:
      - id: deliver-validate
        name: Deliver validation
        entry: deliver
        args: ['--spec', 'deliver.toml', '--strict']
        language: system
        pass_filenames: false
```

### Options

- `--spec`: Path to your deliver spec file (default: `deliver.toml`)
- `--strict`: Exit with error code 1 if any checks fail
- `--base`: Base directory for relative paths (default: `.`)
- `--format`: Output format (text or json)

### Skipping the Hook

To skip the pre-commit hook for a single commit:
```bash
git commit --no-verify -m "Your commit message"
```

## Hook Configuration for Projects

For projects that want to distribute deliver as a pre-commit hook, add the following to your `.pre-commit-hooks.yaml`:

```yaml
repos:
  - repo: local
    hooks:
      - id: deliver-validate
        name: Deliver validation
        entry: deliver
        args: ['--spec', 'deliver.toml', '--strict']
        language: system
        pass_filenames: false
        types: [rust]
```

This allows users to add deliver to their pre-commit configuration with:
```bash
pre-commit install --repo https://github.com/yourusername/deliver
```