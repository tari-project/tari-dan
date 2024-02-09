//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

import { toHexString } from "./helpers";

export class Hash {
  private value: number[];
  constructor(value: number[]) {
    this.value = value;
  }
  toString() {
    return toHexString(this.value);
  }
  toJSON() {
    return this.value;
  }
}

export enum TAG {
  ComponentAddress = 0,
  Metadata = 1,
  NonFungibleAddress = 2,
  ResourceAddress = 3,
  VaultId = 4,
  BucketId = 5,
}

export class Tagged {
  private value: any;
  private tag: number;
  constructor(tag: number, value: any) {
    this.tag = tag;
    this.value = value;
  }
  toString() {
    return this.value.toString();
  }
  toJSON() {
    return { "@@TAGGED@@": [this.tag, this.value] };
  }
}

export class ResourceAddress {
  private tagged: Tagged;
  constructor(value: any) {
    this.tagged = new Tagged(value["@@TAGGED@@"][0], new Hash(value["@@TAGGED@@"][1]));
  }
  toString() {
    return `Resource(${this.tagged.toString()})`;
  }
  toJSON() {
    return this.tagged.toJSON();
  }
}

export class UnclaimedConfidentialOutputAddress {
  private hash: Hash;
  constructor(value: any) {
    this.hash = new Hash(value);
  }
  toString() {
    return `UnclaimedConfidentialOutput(${this.hash.toString()})`;
  }
  toJSON() {
    return this.hash.toJSON();
  }
}

export type u64 = number;
export type u32 = number;
export class U256 {
  private value: number[];
  constructor(value: any) {
    this.value = value;
  }
  toString() {
    return toHexString(this.value);
  }
  toJSON() {
    return this.value;
  }
}

export type NonFungibleIdType = U256 | number | string;

export class NonFungibleId {
  private value: NonFungibleIdType;
  constructor(value: any) {
    if (value.hasOwnProperty("U256")) {
      this.value = new U256(value.U256);
    } else if (value.hasOwnProperty("Uint32")) {
      this.value = value.Uint32;
    } else if (value.hasOwnProperty("Uint64")) {
      this.value = value.Uint64;
    } else if (value.hasOwnProperty("String")) {
      this.value = value;
    } else {
      throw "unimplemented2";
    }
  }
  toString() {
    return this.value.toString();
  }
  toJSON() {
    switch (typeof this.value) {
      case "string":
        return { string: this.value };
      case "number":
        return { Uint64: this.value };
    }
    return { U256: this.value };
  }
}

export class NonFungibleAddressContents {
  private resource_address: ResourceAddress;
  private id: NonFungibleId;
  constructor(value: any) {
    this.resource_address = new ResourceAddress(value.resource_address);
    this.id = new NonFungibleId(value.id);
  }
  toString() {
    return `${this.resource_address.toString()}, ${this.id.toString()}`;
  }
  toJSON() {
    return { resource_address: this.resource_address, id: this.id };
  }
}

export class NonFungibleAddress {
  private tagged: Tagged;
  constructor(value: any) {
    this.tagged = new Tagged(value["@@TAGGED@@"][0], new NonFungibleAddressContents(value["@@TAGGED@@"][1]));
  }
  toString() {
    return `NonFungible(${this.tagged.toString()})`;
  }
  toJSON() {
    return this.tagged.toJSON();
  }
}

export class NonFungibleIndexAddress {
  private resource_address: ResourceAddress;
  private index: number;
  constructor(value: any) {
    this.resource_address = new ResourceAddress(value.resource_address);
    this.index = value.index;
  }
  toString() {
    return `NonFungibleIndex(${this.resource_address.toString()}, ${this.index})`;
  }
  toJSON() {
    return { resource_address: this.resource_address, index: this.index };
  }
}

export class ComponentAddress {
  private tagged: Tagged;
  constructor(value: any) {
    this.tagged = new Tagged(value["@@TAGGED@@"][0], new Hash(value["@@TAGGED@@"][1]));
  }
  toString() {
    return `Component(${this.tagged.toString()})`;
  }
  toJSON() {
    return this.tagged.toJSON();
  }
}

export class VaultId {
  private tagged: Tagged;
  constructor(value: any) {
    this.tagged = new Tagged(value["@@TAGGED@@"][0], new Hash(value["@@TAGGED@@"][1]));
  }
  toString() {
    return `Vault(${this.tagged.toString()})`;
  }
  toJSON() {
    return this.tagged.toJSON();
  }
}

export type SubstateIdType =
  | ResourceAddress
  | ComponentAddress
  | VaultId
  | UnclaimedConfidentialOutputAddress
  | NonFungibleAddress
  | NonFungibleIndexAddress;

export class SubstateId {
  private value: SubstateIdType;
  constructor(value: any) {
    if (value.hasOwnProperty("Component")) {
      this.value = new ComponentAddress(value.ComponentAddress);
    } else if (value.hasOwnProperty("Resource")) {
      this.value = new ResourceAddress(value.Resource);
    } else if (value.hasOwnProperty("Vault")) {
      this.value = new VaultId(value.Vault);
    } else if (value.hasOwnProperty("UnclaimedConfidentialOutput")) {
      this.value = new UnclaimedConfidentialOutputAddress(value.UnclaimedConfidentialOutput);
    } else if (value.hasOwnProperty("NonFungible")) {
      this.value = new NonFungibleAddress(value.NonFungible);
    } else if (value.hasOwnProperty("NonFungibleIndex")) {
      this.value = new NonFungibleIndexAddress(value.NonFungibleIndex);
    } else {
      throw "unimplemented";
    }
  }
  toString() {
    return this.value.toString();
  }
  toJSON() {
    if (this.value instanceof ComponentAddress) {
      return { Component: this.value };
    } else if (this.value instanceof ResourceAddress) {
      return { Resource: this.value };
    } else if (this.value instanceof VaultId) {
      return { Vault: this.value };
    } else if (this.value instanceof UnclaimedConfidentialOutputAddress) {
      return { UnclaimedConfidentialOutput: this.value };
    } else if (this.value instanceof NonFungibleAddress) {
      return { NonFungible: this.value };
    } else if (this.value instanceof NonFungibleIndexAddress) {
      return { NonFungibleIndex: this.value };
    }
    throw "Unknown type";
  }
}

export class TariPermissionAccountBalance {
  private value: SubstateId;
  constructor(value: any) {
    this.value = new SubstateId(value);
  }
  toString() {
    return `AccountBalance(${this.value.toString()})`;
  }
  toJSON() {
    return { AccountBalance: this.value };
  }
}

export class TariPermissionAccountInfo {
  constructor() {}
  toString() {
    return `AccountInfo`;
  }
  toJSON() {
    return "AccountInfo";
  }
}

export class TariPermissionAccountList {
  private value?: ComponentAddress;
  constructor(value?: any) {
    this.value = new ComponentAddress(value["@@TAGGED@@"][1]);
  }
  toString() {
    if (this.value !== null) {
      return `AccountList(${this.value?.toString()})`;
    } else {
      return "AccountList(any)";
    }
  }
  toJSON() {
    if (this.value === undefined) {
      return { AccountList: this.value };
    } else {
      return { AccountList: null };
    }
  }
}

export class TariPermissionKeyList {
  constructor() {}
  toString() {
    return `KeyList`;
  }
  toJSON() {
    return "KeyList";
  }
}

export class TariPermissionTransactionGet {
  constructor() {}
  toString() {
    return `TransactionGet`;
  }
  toJSON() {
    return "TransactionGet";
  }
}
export class TariPermissionTransactionSend {
  private value?: SubstateId;
  constructor(value?: SubstateId) {
    this.value = value;
  }
  toString() {
    if (this.value === undefined) {
      return "TransactionSend(any)";
    } else {
      return `TransactionSend(${this.value?.toString()})`;
    }
  }
  toJSON() {
    if (this.value === undefined) {
      return { TransactionSend: null };
    } else {
      return { TransactionSend: this.value };
    }
  }
}

export class TariPermissionGetNft {
  private value0?: SubstateId;
  private value1?: ResourceAddress;
  constructor(value0?: SubstateId, value1?: ResourceAddress) {
    this.value0 = value0;
    this.value1 = value1;
  }
  toString() {
    let svalue0, svalue1;
    if (this.value0) {
      svalue0 = this.value0.toString();
    } else {
      svalue0 = "any";
    }
    if (this.value1) {
      svalue1 = this.value1.toString();
    } else {
      svalue1 = "any";
    }
    return `GetNft(${svalue0},${svalue1})`;
  }
  toJSON() {
    return { GetNft: [this.value0, this.value1] };
  }
}

export class TariPermissionNftGetOwnershipProof {
  private value?: ResourceAddress;
  constructor(value?: ResourceAddress) {
    this.value = value;
  }
  toString() {
    if (this.value) {
      return `NftGetOwnershipProof(${this.value?.toString()})`;
    } else {
      return `NftGetOwnershipProof(any)`;
    }
  }
  toJSON() {
    return { NftGetOwnershipProof: this.value };
  }
}

export type TariPermission =
  | TariPermissionNftGetOwnershipProof
  | TariPermissionAccountBalance
  | TariPermissionAccountInfo
  | TariPermissionAccountList
  | TariPermissionKeyList
  | TariPermissionTransactionGet
  | TariPermissionTransactionSend
  | TariPermissionGetNft;

export class TariPermissions {
  private permissions: TariPermission[];

  constructor() {
    this.permissions = [];
  }

  addPermission(permission: TariPermission) {
    this.permissions.push(permission);
  }

  toJSON() {
    return this.permissions;
  }
}

export function parse(permission: any) {
  if (permission.hasOwnProperty("AccountBalance")) {
    return new TariPermissionAccountBalance(permission.AccountBalance);
  } else if (permission === "AccountInfo") {
    return new TariPermissionAccountInfo();
  } else if (permission.hasOwnProperty("AccountList")) {
    return new TariPermissionAccountList(permission.AccountList);
  } else if (permission == "KeyList") {
    return new TariPermissionKeyList();
  } else if (permission.hasOwnProperty("TransactionSend")) {
    return new TariPermissionTransactionSend(permission.TransactionSend);
  } else if (permission === "TransactionGet") {
    return new TariPermissionTransactionGet();
  } else if (permission.hasOwnProperty("GetNft")) {
    return new TariPermissionGetNft(permission.GetNft);
  } else if (permission.hasOwnProperty("NftGetOwnershipProof")) {
    return new TariPermissionNftGetOwnershipProof(permission.NftGetOwnershipProof);
  }
  return null;
}
