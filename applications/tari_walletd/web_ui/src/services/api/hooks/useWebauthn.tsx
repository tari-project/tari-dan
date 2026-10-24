// Copyright 2025 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

import { useMutation, useQuery } from "@tanstack/react-query";
import queryClient from "@api/queryClient";
import {
  webauthnAlreadyRegistered,
  webauthnListPasskeys,
  webauthnRemovePasskey,
  webauthnRenamePasskey,
} from "@utils/json_rpc";

export const WEBAUTHN_PASSKEYS_QUERY_KEY = ["webauthn_passkeys"];

export const useWebauthnAlreadyRegistered = () => {
  return useQuery({
    queryKey: ["webauthn_already_registered"],
    queryFn: () => {
      return webauthnAlreadyRegistered();
    },
    refetchInterval: false,
    notifyOnChangeProps: ["data", "error"],
    retryOnMount: false,
    retry: false,
  });
};

export const useWebauthnListPasskeys = () => {
  return useQuery({
    queryKey: WEBAUTHN_PASSKEYS_QUERY_KEY,
    queryFn: () => webauthnListPasskeys({}),
  });
};

export const useWebauthnRenamePasskey = () => {
  return useMutation({
    mutationFn: ({ id, label }: { id: number; label: string }) => webauthnRenamePasskey({ id, label }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: WEBAUTHN_PASSKEYS_QUERY_KEY });
    },
  });
};

export const useWebauthnRemovePasskey = () => {
  return useMutation({
    mutationFn: (id: number) => webauthnRemovePasskey({ id }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: WEBAUTHN_PASSKEYS_QUERY_KEY });
    },
  });
};
