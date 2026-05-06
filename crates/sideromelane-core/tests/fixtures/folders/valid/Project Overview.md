---
title: Project Overview
tags: [planning, product]
status: active
summary: "foo: bar"
---

# Project Overview

This note exercises all the main parsing paths.

## Goals

See [[Roadmap]] for the high-level plan and [[Release Checklist]] for milestones.

![[architecture.png]]

## Implementation

The following code block contains wiki-link and heading syntax that must NOT be extracted:

```rust
// [[fake_link]] should not appear in wiki_links
let x = "[[another_fake]]";
// # fake heading should not appear in headings
println!("done");
```

After the fence, real links resume: [[Appendix]].

## Notes

~~~python
# [[also_fake]] inside tilde fence
# # fake tilde heading
print("hello")
~~~

End of document.
