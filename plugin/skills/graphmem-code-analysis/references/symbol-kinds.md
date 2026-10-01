# Symbol kinds and names by language

Use these values for `find_symbol`'s `kind` filter and to read `code_outline`
results. `import` is emitted by every language that has imports, and it is
returned by `find_symbol` only when `kind: "import"` is passed.

| Language | Kinds | Naming notes |
| --- | --- | --- |
| Rust | `module`, `struct`, `enum`, `enumMember`, `field`, `function`, `method`, `interface` (traits), `impl`, `constant`, `variable`, `import` | an `impl` block is named by its type (`Invoice`) and holds its methods |
| Go | `struct`, `interface`, `field`, `function`, `method`, `constant`, `variable`, `typeParameter`, `import` | |
| C | `struct`, `enum`, `enumMember`, `field`, `function`, `variable`, `import` | |
| C++ (and `.h`) | `namespace`, `class`, `struct`, `constructor`, `method`, `function`, `field`, `enum`, `enumMember`, `import` | functions nest under their `namespace` |
| Python | `class`, `method`, `function`, `constant`, `variable`, `field`, `import` | |
| JavaScript/JSX | `class`, `constructor`, `method`, `function`, `field`, `constant`, `variable`, `import` | `export default { ... }` is `object default`, with its function members as `method` |
| TypeScript/TSX | the JavaScript kinds plus `interface`, `enum`, `enumMember` | |
| Ruby | `module`, `class`, `method`, `function`, `constant`, `import` | |
| PHP | `class`, `interface`, `enum`, `enumMember`, `method`, `function`, `property`, `import` | |
| Elixir | `module`, `impl`, `function`, `macro`, `guard`, `import` | functions are `name/arity`, consecutive clauses merge into one symbol; nested modules use the full name (`Parent.Child`); `alias`/`import`/`require`/`use` are `import` |
| HEEx (and `~H`) | `component`, `slot` | usages, not definitions: `.button`, `Layouts.app`, `:subtitle` |
| EEx | `expression` | each `<% %>` directive; coverage is always partial |
| SQL | the object type of each `CREATE`: `table`, `view`, `index`, `function`, `trigger`, `type`, ... | named as written, so a schema-qualified `public.users` is found by `users` |
| Zig | `struct`, `enum`, `union`, `opaque`, `error_set`, `field`, `function`, `test`, `import` | a `const` is a symbol only when its value is a container or `@import` |
| Racket | `module`, `struct`, `function`, `macro`, `constant`, `import` | only module-level forms (inside `module`/`module+`/`begin`) are listed, not definitions nested in function bodies; `.rkt` and `.rktl` files |
| Bash | `function` | |
| CSS/SCSS | `selector`, `media`, `supports`, `keyframes`, `mixin`, `function`, `variable`, `import` | a rule is named by its whole selector list (`.btn, .btn-primary`); `@media` by its query; variables are `--custom` properties and `$scss` variables; `@include` is not listed |
