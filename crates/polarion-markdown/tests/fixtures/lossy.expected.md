# Lossy constructs

Soft line breaks\
become hard breaks, and two spaces\
also end a line.

A footnote reference^\[1\]^ becomes a superscript.

**Term**

> Definition of the term.

> [!WARNING] Heads up
> A custom alert title.

> [!NOTE]
> A note written as a directive.

Other directives lose their wrapper.

| Left | Plain |
| --- | --- |
| a | b |

| One | Two |
| --- | --- |
| short row |  |

A loose list of single paragraphs:

- first
- second

# Setext heading

## Closed heading

**bold** and *italic* with underscores.

- star bullet
- list

Numbering:

1. parenthesis
2. delimiter

Tasks:

- [x] upper-case task mark

An indented code block:

```
indented code
```

```
tilde fence
```

-----

Two spaces and a collapse.

Unneeded escapes vanish: a.b and (x).

$$x^2$$

```math
a
b
```

Entities: © & \<

[reference link](https://example.com/ref "Ref title"), <https://example.com/bare>, [www.example.com](http://www.example.com) and <someone@example.com>.

![alt with code](https://example.com/a.png)

Raw tags: **raw bold**, *raw italic* and a\
break.

# Heading with bold

```rust
fn main() {}
```

-----

^\[1\]^ The footnote text.
