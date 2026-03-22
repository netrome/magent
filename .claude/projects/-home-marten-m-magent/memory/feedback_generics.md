---
name: Prefer generics over trait objects
description: When integrating trait-based components, use impl Trait / generics rather than Arc<dyn Trait> or Box<dyn Trait>
type: feedback
---

Prefer generics (`impl LlmClient`) over trait objects (`Arc<dyn LlmClient>`) when wiring together components.

**Why:** User finds trait objects ("some `Arc<dyn ...>` thingamajig") overly complex for this codebase. Generics are simpler and more idiomatic here.

**How to apply:** When defining functions or structs that depend on a trait, use generic type parameters or `impl Trait` in argument position. Reserve `dyn Trait` for cases where dynamic dispatch is genuinely needed (e.g., heterogeneous collections).
