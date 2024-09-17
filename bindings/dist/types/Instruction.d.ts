import type { Amount } from "./Amount";
import type { Arg } from "./Arg";
import type { ComponentAccessRules } from "./ComponentAccessRules";
import type { ComponentAddress } from "./ComponentAddress";
import type { ConfidentialClaim } from "./ConfidentialClaim";
import type { LogLevel } from "./LogLevel";
import type { OwnerRule } from "./OwnerRule";
import type { ResourceAddress } from "./ResourceAddress";
export type Instruction = {
    CreateAccount: {
        public_key_address: string;
        owner_rule: OwnerRule | null;
        access_rules: ComponentAccessRules | null;
        workspace_bucket: string | null;
    };
} | {
    CallFunction: {
        template_address: Uint8Array;
        function: string;
        args: Array<Arg>;
    };
} | {
    CallMethod: {
        component_address: ComponentAddress;
        method: string;
        args: Array<string>;
    };
} | {
    PutLastInstructionOutputOnWorkspace: {
        key: Array<number>;
    };
} | {
    EmitLog: {
        level: LogLevel;
        message: string;
    };
} | {
    ClaimBurn: {
        claim: ConfidentialClaim;
    };
} | {
    ClaimValidatorFees: {
        epoch: number;
        validator_public_key: string;
    };
} | "DropAllProofsInWorkspace" | {
    AssertBucketContains: {
        key: Array<number>;
        resource_address: ResourceAddress;
        min_amount: Amount;
    };
};
