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

import { useMutation, useQuery } from '@tanstack/react-query';
import { jsonRpc } from '../../utils/json_rpc';
import { apiError } from '../helpers/types';
import queryClient from '../queryClient';

//   Fees are passed as strings because Amount is tagged
export const useAccountsClaimBurn = (
  account: string,
  claimProof: any,
  fee: number
) => {
  return useMutation(
    () => {
      return jsonRpc('accounts.claim_burn', {
        account,
        claim_proof: claimProof,
        fee: fee,
      });
    },
    {
      onError: (error: apiError) => {
        error;
      },
      onSettled: () => {
        queryClient.invalidateQueries(['accounts']);
      },
    }
  );
};

export const useAccountsCreate = (
  accountName: string | undefined,
  signingKeyIndex: any | undefined,
  customAccessRules: any | undefined,
  fee: number | undefined,
  is_default: boolean | false
) => {
  return useMutation(
    () => {
      return jsonRpc('accounts.create', [
        accountName,
        signingKeyIndex,
        customAccessRules,
        fee,
        is_default,
      ]);
    },
    {
      onError: (error: apiError) => {
        error;
      },
      onSettled: () => {
        queryClient.invalidateQueries(['accounts']);
      },
    }
  );
};

export const useAccountsCreateFreeTestCoins = () => {
  const createFreeTestCoins = async ({
    accountName,
    amount,
    fee,
  }: {
    accountName: string | undefined;
    amount: number | undefined;
    fee: number | undefined;
  }) => {
    const result = await jsonRpc('accounts.create_free_test_coins', [
      { Name: accountName },
      amount,
      fee,
    ]);
    return result;
  };

  return useMutation(createFreeTestCoins, {
    onError: (error: apiError) => {
      console.error(error);
    },
    onSettled: () => {
      queryClient.invalidateQueries(['transactions']);
      queryClient.invalidateQueries(['accounts_balances']);
    },
  });
};

export const useAccountsList = (offset: number, limit: number) => {
  return useQuery({
    queryKey: ['accounts'],
    queryFn: () => {
      return jsonRpc('accounts.list', [offset, limit]);
    },
    onError: (error: apiError) => {
      error;
    },
  });
};

export const useAccountsInvoke = (
  accountName: string,
  method: string,
  args: any[]
) => {
  return useQuery({
    queryKey: ['accounts_invoke'],
    queryFn: () => {
      return jsonRpc('accounts.invoke', [accountName, method, args]);
    },
    onError: (error: apiError) => {
      error;
    },
  });
};

export const useAccountsGetBalances = (accountName: string) => {
  return useQuery({
    queryKey: ['accounts_balances'],
    queryFn: () => {
      return jsonRpc('accounts.get_balances', [accountName]);
    },
    onError: (error: apiError) => {
      error;
    },
  });
};

export const useAccountsGet = (nameOrAddress: string) => {
  return useQuery({
    queryKey: ['accounts_get'],
    queryFn: () => {
      return jsonRpc('accounts.get', [nameOrAddress]);
    },
    onError: (error: apiError) => {
      error;
    },
  });
};

export const useAccountNFTsList = (offset: number, limit: number) => {
  return useQuery({
    queryKey: ['nfts_list'],
    queryFn: () => {
      return jsonRpc('nfts.list', [offset, limit]);
    },
    onError: (error: apiError) => {
      error;
    },
  });
};
