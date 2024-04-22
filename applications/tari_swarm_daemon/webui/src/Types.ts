//  Copyright 2024 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause


export interface ExecutedTransaction {
  transaction: Transaction
  abort_details: string | null,
  final_decision: string | null,
}

export interface Transaction {
  id: string,
}
