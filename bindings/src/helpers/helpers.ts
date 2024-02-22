//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

import { JrpcPermission } from "../types/JrpcPermission";
import { RejectReason } from "../types/RejectReason";
import { SubstateDiff } from "../types/SubstateDiff";
import { SubstateId } from "../types/SubstateId";
import { TransactionResult } from "../types/TransactionResult";

export function substateIdToString(substateId: SubstateId | null): string {
  if (substateId === null) {
    return "";
  }
  if ("Component" in substateId) {
    return substateId.Component;
  }
  if ("Resource" in substateId) {
    return substateId.Resource;
  }
  if ("Vault" in substateId) {
    return substateId.Vault;
  }
  if ("UnclaimedConfidentialOutput" in substateId) {
    return substateId.UnclaimedConfidentialOutput;
  }
  if ("NonFungible" in substateId) {
    return substateId.NonFungible;
  }
  if ("NonFungibleIndex" in substateId) {
    return `${substateId.NonFungibleIndex.resource_address}:${substateId.NonFungibleIndex.index}`;
  }
  if ("TransactionReceipt" in substateId) {
    return substateId.TransactionReceipt;
  }
  if ("FeeClaim" in substateId) {
    return substateId.FeeClaim;
  }
  console.error("Unknown substate id", substateId);
  return "Unknown";
}

export function stringToSubstateId(substateId: string): SubstateId {
  const parts = substateId.split("_", 2);
  if (parts.length !== 2) {
    throw new Error(`Invalid substate id: ${substateId}`);
  }

  switch (parts[0]) {
    case "component":
      return { Component: parts[1] };
    case "resource":
      return { Resource: parts[1] };
    case "vault":
      return { Vault: parts[1] };
    case "commitment":
      return { UnclaimedConfidentialOutput: parts[1] };
    case "nft":
      return { NonFungible: parts[1] };
    case "txreceipt":
      return { TransactionReceipt: parts[1] };
    case "feeclaim":
      return { FeeClaim: parts[1] };
    default:
      throw new Error(`Unknown substate id: ${substateId}`);
  }
}

export function rejectReasonToString(reason: RejectReason | null): string {
  if (reason === null) {
    return "";
  }
  if (typeof reason === "string") {
    return reason;
  }
  if ("ShardsNotPledged" in reason) {
    return `ShardsNotPledged(${reason.ShardsNotPledged})`;
  }
  if ("ExecutionFailure" in reason) {
    return `ExecutionFailure(${reason.ExecutionFailure})`;
  }
  if ("ShardPledgedToAnotherPayload" in reason) {
    return `ShardPledgedToAnotherPayload(${reason.ShardPledgedToAnotherPayload})`;
  }
  if ("ShardRejected" in reason) {
    return `ShardRejected(${reason.ShardRejected})`;
  }
  if ("FeesNotPaid" in reason) {
    return `FeesNotPaid(${reason.FeesNotPaid})`;
  }
  console.error("Unknown reason", reason);
  return "Unknown";
}

export function getSubstateDiffFromTransactionResult(result: TransactionResult): SubstateDiff | null {
  if ("Accept" in result) {
    return result.Accept;
  }
  if ("AcceptFeeRejectRest" in result) {
    return result.AcceptFeeRejectRest[0];
  }
  return null;
}

export function getRejectReasonFromTransactionResult(result: TransactionResult): RejectReason | null {
  if ("Reject" in result) {
    return result.Reject;
  }
  if ("AcceptFeeRejectRest" in result) {
    return result.AcceptFeeRejectRest[1];
  }
  return null;
}

export function jrpcPermissionToString(jrpcPermission: JrpcPermission): string {
  if (typeof jrpcPermission === "string") {
    return jrpcPermission;
  }
  if ("NftGetOwnershipProof" in jrpcPermission) {
    return `NftGetOwnershipProof(${jrpcPermission.NftGetOwnershipProof})`;
  }
  if ("AccountBalance" in jrpcPermission) {
    return `AccountBalance(${substateIdToString(jrpcPermission.AccountBalance)})`;
  }
  if ("AccountList" in jrpcPermission) {
    return `AccountList(${jrpcPermission.AccountList})`;
  }
  if ("TransactionSend" in jrpcPermission) {
    return `TransactionSend(${jrpcPermission.TransactionSend})`;
  }
  if ("GetNft" in jrpcPermission) {
    return `GetNft(${substateIdToString(jrpcPermission.GetNft[0])}, ${jrpcPermission.GetNft[1]})`;
  }
  return "Unknown";
}
