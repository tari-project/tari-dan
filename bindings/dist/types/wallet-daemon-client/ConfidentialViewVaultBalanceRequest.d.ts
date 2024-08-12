import type { VaultId } from "../VaultId";
export interface ConfidentialViewVaultBalanceRequest {
    vault_id: VaultId;
    minimum_expected_value: number | null;
    maximum_expected_value: number | null;
    view_key_id: number;
}
