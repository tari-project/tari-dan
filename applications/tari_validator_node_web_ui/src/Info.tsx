import { useState } from "react";
import { IEpoch, IIdentity } from "./interfaces";
import { registerValidatorNode } from "./json_rpc";

function Info({ epoch, identity, shardKey }: { epoch: IEpoch; identity: IIdentity; shardKey: string | null }) {
  const [registering, setRegistering] = useState(false);
  const [registerMessage, setRegisterMessage] = useState("");
  const register = () => {
    setRegistering(true);
    registerValidatorNode().then((response) => {
      if (response.message) {
        setRegistering(false);
        setRegisterMessage(response.message);
      } else {
        setRegisterMessage(`Registration successul, the TxId ${response.transaction_id}`);
      }
    });
  };
  const renderShardKey = () => {
    if (shardKey === null)
      return (
        <>
          <div className="label">Shard key</div>
          <div>
            <span
              className={`${registering ? "disabled-button" : "button"}`}
              id="register"
              onClick={registering ? () => {} : register}
            >
              Register
            </span>
            {registerMessage ? <span>{registerMessage}</span> : null}
          </div>
        </>
      );
    return (
      <>
        <div className="label">Shard key</div>
        <div className="key">{shardKey}</div>
      </>
    );
  };
  return (
    <div className="info">
      <div className="label">Epoch</div>
      <div className="">
        {epoch.current_epoch} ({epoch.is_valid ? "Valid" : "Not valid"})
      </div>
      <div className="label">Node id</div>
      <div className="key">{identity.node_id}</div>
      <div className="label">Public address</div>
      <div className="key">{identity.public_address}</div>
      <div className="label">Public key</div>
      <div className="key">{identity.public_key}</div>
      {renderShardKey()}
    </div>
  );
}

export default Info;
