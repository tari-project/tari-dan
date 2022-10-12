interface IEpoch {
  current_epoch: number;
  is_valid: boolean;
}

interface IIdentity {
  node_id: string;
  public_address: string;
  public_key: string;
}

export { type IEpoch, type IIdentity };
