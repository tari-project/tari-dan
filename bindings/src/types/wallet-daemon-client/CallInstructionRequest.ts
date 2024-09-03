// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.
import type { ComponentAddressOrName } from "./ComponentAddressOrName";
import type { Instruction } from "../Instruction";
import type { SubstateRequirement } from "../SubstateRequirement";

export interface CallInstructionRequest { instructions: Array<Instruction>, fee_account: ComponentAddressOrName, dump_outputs_into: ComponentAddressOrName | null, max_fee: number, inputs: Array<SubstateRequirement>, override_inputs: boolean | null, new_outputs: number | null, is_dry_run: boolean, proof_ids: Array<number>, min_epoch: number | null, max_epoch: number | null, }