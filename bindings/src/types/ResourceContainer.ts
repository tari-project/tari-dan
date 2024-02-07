// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.
import type { NonFungibleId } from "./NonFungibleId";
import type { ResourceAddress } from "./ResourceAddress";

export type ResourceContainer =
  | { Fungible: { address: ResourceAddress; amount: number; locked_amount: number } }
  | {
      NonFungible: {
        address: ResourceAddress;
        token_ids: Array<NonFungibleId>;
        locked_token_ids: Array<NonFungibleId>;
      };
    }
  | {
      Confidential: {
        address: ResourceAddress;
        commitments: Record<string, ConfidentialOutput>;
        revealed_amount: number;
        locked_commitments: Record<string, ConfidentialOutput>;
        locked_revealed_amount: number;
      };
    };
