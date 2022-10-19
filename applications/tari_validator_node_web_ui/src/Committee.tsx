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
