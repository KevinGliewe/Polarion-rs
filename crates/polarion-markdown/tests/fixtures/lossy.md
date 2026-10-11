# Lossy constructs

Soft line breaks
become hard breaks, and two spaces  
also end a line.

A footnote reference[^1] becomes a superscript.

[^1]: The footnote text.

Term

: Definition of the term.

> [!WARNING] Heads up
> A custom alert title.

:::note
A note written as a directive.
:::

:::details
Other directives lose their wrapper.
:::

| Left | Plain |
| :-- | --- |
| a | b |

| One | Two |
| --- | --- |
| short row |

A loose list of single paragraphs:

- first

- second

Setext heading
==============

## Closed heading ##

__bold__ and _italic_ with underscores.

* star bullet
* list

Numbering:

1) parenthesis
1) delimiter

Tasks:

- [X] upper-case task mark

An indented code block:

    indented code

~~~
tilde fence
~~~

***

Two  spaces and a <!-- comment --> collapse.

<!-- a comment block -->

Unneeded escapes vanish: a\.b and \(x\).

```math
x^2
```

$$a
b$$

Entities: &copy; &amp; &lt;

[reference link][ref], https://example.com/bare, www.example.com and someone@example.com.

[ref]: https://example.com/ref "Ref title"

![alt *with* `code`](https://example.com/a.png)

Raw tags: <b>raw bold</b>, <i>raw italic</i> and a<br>break.

# Heading with **bold**

```rust title="main.rs"
fn main() {}
```
