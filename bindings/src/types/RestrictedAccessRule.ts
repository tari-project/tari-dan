// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.
import type { RequireRule } from "./RequireRule";

export type RestrictedAccessRule = { "Require": RequireRule } | { "AnyOf": Array<RestrictedAccessRule> } | { "AllOf": Array<RestrictedAccessRule> };