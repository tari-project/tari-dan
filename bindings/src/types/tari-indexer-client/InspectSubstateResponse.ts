// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.
import type { Substate } from "../Substate";
import type { SubstateId } from "../SubstateId";

export interface InspectSubstateResponse { address: SubstateId, version: number, substate: Substate, created_by_transaction: string, }