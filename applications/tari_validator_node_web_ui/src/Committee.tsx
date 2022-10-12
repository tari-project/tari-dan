import React, { useState } from "react";

function Committee({
  begin,
  end,
  members,
  publicKey,
}: {
  begin: string;
  end: string;
  members: Array<string>;
  publicKey: string;
}) {
  const [visible, setVisible] = useState(false);
  const toggle = (event: React.MouseEvent<HTMLDivElement>) => {
    setVisible(!visible);
  };

  return (
    <div className="committee-wrapper">
      <div className="button" onClick={toggle}>
        {visible ? "Hide members" : "Show  members"}
      </div>
      <div className="committee">
        <div className="committee-range">
          <div className="range-label label">Range</div>
          {end < begin ? (
            <>
              <div className="range">
                [<span className="key">{begin}</span>,{" "}
                <span className="key">ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff</span>]
              </div>
              <div className="range">
                [<span className="key">0000000000000000000000000000000000000000000000000000000000000000</span>,{" "}
                <span className="key">{end}</span>]
              </div>
            </>
          ) : (
            <div>
              [<span className="key">{begin}</span>, <span className="key">{end}</span>]
            </div>
          )}
        </div>
        {visible ? (
          <div className="members">
            {members.map((member) => (
              <React.Fragment key={member}>
                <div className="label">Public key </div>
                <div className={`member key ${member === publicKey ? "me" : ""}`}>{member}</div>
              </React.Fragment>
            ))}
          </div>
        ) : (
          <></>
        )}
      </div>
    </div>
  );
}

export default Committee;
