---
name: rs-hack-firkin-refactor
description: Use rs-hack for large Rust refactors in Firkin/Cube, especially moving the CubeAPI Firkin backend into firkin-runtime::single_node with AST-aware discovery, batch transforms, dry-run/apply/revert discipline, and compile gates.
---

# rs-hack Firkin Refactor

Use this skill when refactoring Rust code in:

- `/Users/darin/vendor/github.com/apple/containerization`
- `/Users/darin/vendor/github.com/TencentCloud/CubeSandbox`

The goal is to make rs-hack do the repetitive AST work. Do not fall back to
`sed`, Perl, Python regex rewrites, or broad text replacement for Rust code.

## Tool Path

Use the local checkout:

```bash
RSH=/Users/darin/vendor/github.com/1e1f/rs-hack/target/debug/rs-hack
```

If that binary is missing, build it:

```bash
cargo build --manifest-path /Users/darin/vendor/github.com/1e1f/rs-hack/Cargo.toml \
  -p rs-hack
```

## Core Rules

Always use this loop:

1. `find`, `summary`, `impls`, `match-audit`, `neighbors`, or `doc-coverage`.
2. Dry-run with `--format diff` or `--format summary`.
3. Apply with `--apply --local-state`.
4. Capture the run id with `history --local-state`.
5. Run the relevant `cargo check` or test.
6. Revert with `revert --local-state <run-id>` if the compiler exposes a bad
   assumption.

Use `--limit N` to test a risky operation on one or two matches before applying
globally.

## What rs-hack Should Own

Use rs-hack for:

- discovery of definitions and expression sites;
- function and trait-method renames;
- enum variant renames across definitions, patterns, constructors, and usages;
- struct field add/update/remove across definitions and literals;
- enum-variant struct literal changes using `Enum::Variant` targets;
- adding use statements;
- adding derives;
- adding impl methods;
- adding, updating, or removing function/method call arguments;
- adding, updating, or removing match arms;
- auditing enum match exhaustiveness;
- replacing, removing, or commenting AST expression nodes with `transform`;
- batch JSON/YAML operation specs.

Do not use rs-hack for:

- physically moving files;
- deciding crate boundaries;
- semantic type analysis;
- non-Rust files;
- one-off edits where a normal patch is clearer.

## Discovery Commands

```bash
$RSH summary --path path/to/file.rs
$RSH neighbors --path path/to/file.rs
$RSH impls --paths "src/**/*.rs" --trait TraitName
$RSH match-audit --paths "src/**/*.rs" --enum EnumName
$RSH doc-coverage --paths "src/**/*.rs" --fields
$RSH find --paths "src/**/*.rs" --name Symbol --format locations
$RSH find --paths "src/**/*.rs" --node-type struct-literal --name TypeName --format snippets
$RSH find --paths "src/**/*.rs" --field-name field_name --format locations
$RSH find --paths "src/**/*.rs" --name Symbol --format json > /tmp/symbols.json
```

Use `--kind` for broad semantic groups and `--node-type` for exact AST shapes.

Common `--kind` values:

- `struct`: definitions and struct literals.
- `function`: functions, trait methods, impl methods, calls, method calls.
- `enum`: enum definitions and usages.
- `trait`: traits and trait impls.

Common `--node-type` values:

- `struct`
- `struct-literal`
- `type-ref`
- `function`
- `function-call`
- `method-call`
- `impl-method`
- `trait-impl`
- `enum`
- `enum-usage`
- `match-arm`
- `macro-call`

## Mechanical Operations

Rename functions or trait methods:

```bash
$RSH rename --local-state --paths "src/**/*.rs" \
  --name old_name \
  --to new_name \
  --kind function \
  --format diff
```

Rename enum variants:

```bash
$RSH rename --local-state --paths "src/**/*.rs" \
  --name EnumName::OldVariant \
  --to NewVariant \
  --format diff
```

rs-hack does not currently have a first-class struct/type rename that updates
definitions, type refs, constructor paths, and struct literal paths together.
For type renames, move/rename declarations with the editor, then use rs-hack to
inventory and repair safe AST sites.

Inventory type references and constructors:

```bash
$RSH find --paths "src/**/*.rs" --node-type type-ref --name OldType --format locations
$RSH find --paths "src/**/*.rs" --node-type struct-literal --name OldType --format locations
$RSH find --paths "src/**/*.rs" --node-type identifier --name OldType --format locations
```

Replace type references:

```bash
$RSH transform --local-state --paths "src/**/*.rs" \
  --node-type type-ref \
  --name OldType \
  --action replace \
  --with NewType \
  --format diff
```

Do not replace a whole `struct-literal` node with a bare type name; that deletes
the literal fields. Use the struct-literal inventory to guide manual movement
or a narrower custom transform.

Add a field to definition and all literals:

```bash
$RSH add --local-state --paths "src/**/*.rs" \
  --name TypeName \
  --field-name new_field \
  --field-type "Option<Value>" \
  --field-value "None" \
  --kind struct \
  --format diff
```

Remove a field from literals only:

```bash
$RSH remove --local-state --paths "src/**/*.rs" \
  --name TypeName \
  --field-name old_field \
  --literal-only \
  --format diff
```

Change call signatures:

```bash
$RSH add --local-state --paths "src/**/*.rs" \
  --call function_or_method \
  --arg "new_arg" \
  --arg-position last \
  --call-type method \
  --format diff

$RSH update --local-state --paths "src/**/*.rs" \
  --call function_or_method \
  --arg-index 1 \
  --arg "replacement_arg" \
  --call-type method \
  --content-filter "OldType" \
  --format diff

$RSH remove --local-state --paths "src/**/*.rs" \
  --call function_or_method \
  --arg-index 2 \
  --call-type function \
  --format diff
```

Add missing match arms:

```bash
$RSH add --local-state --paths "src/**/*.rs" \
  --auto-detect \
  --enum-name EnumName \
  --body "todo!()" \
  --format diff
```

Add imports:

```bash
$RSH add --local-state --paths "src/**/*.rs" \
  --use "crate::single_node::model::SingleNodeCreateRequest" \
  --format diff
```

## Batch Specs

Use batch specs after proving each operation shape with a dry run.

```yaml
base_path: /absolute/path/to/src
operations:
  - type: Transform
    node_type: type-ref
    name_filter: OldType
    action:
      Replace:
        with: NewType
  - type: RenameFunction
    old_name: old_fn
    new_name: new_fn
  - type: AddStructField
    struct_name: CreateRequest
    field_def: "runtime_identity: Option<RuntimeIdentity>"
    literal_default: "None"
    position: Last
```

Run:

```bash
$RSH batch --local-state --spec /tmp/refactor.yaml --format diff
$RSH batch --local-state --spec /tmp/refactor.yaml --apply
```

## Firkin/Cube Extraction Recipe

1. Inventory Cube:

```bash
CUBE=/Users/darin/vendor/github.com/TencentCloud/CubeSandbox/CubeAPI/src
$RSH summary --path "$CUBE/services/sandboxes.rs"
$RSH summary --path "$CUBE/services/templates.rs"
$RSH impls --paths "$CUBE/**/*.rs" --trait SandboxBackend
$RSH find --paths "$CUBE/**/*.rs" --name FirkinAppleVz --format locations
```

2. Create a Cube-local temporary module with normal file edits.

3. Use rs-hack for neutral renames and model cleanup while the code still
   compiles in Cube.

4. Copy the module to:

```text
/Users/darin/vendor/github.com/apple/containerization/crates/runtime/src/single_node/
```

5. Use rs-hack in Firkin to replace Cube types:

```bash
FIRKIN=/Users/darin/vendor/github.com/apple/containerization/crates/runtime/src
$RSH find --paths "$FIRKIN/single_node/**/*.rs" --name AppError --format locations
$RSH transform --local-state --paths "$FIRKIN/single_node/**/*.rs" \
  --node-type type-ref --name AppError --action replace --with Error --format diff
```

6. Compile after every applied family of operations.

## Revert

```bash
$RSH history --local-state --limit 10
$RSH revert --local-state <run-id>
```

If an operation is too broad, revert immediately and re-run with
`--node-type`, `--content-filter`, `--exclude`, or `--limit`.
