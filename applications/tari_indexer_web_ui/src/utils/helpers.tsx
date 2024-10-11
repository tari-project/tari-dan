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

import { toHexString } from "../routes/VN/Components/helpers";

const renderJson = (json: any) => {
  if (Array.isArray(json)) {
    if (json.length === 32) {
      return <span className="string">"{toHexString(json)}"</span>;
    }
    return (
      <>
        [
        <ol>
          {json.map((val) => (
            <li>{renderJson(val)},</li>
          ))}
        </ol>
        ],
      </>
    );
  } else if (typeof json === "object" && json !== null) {
    return (
      <>
        {"{"}
        <ul>
          {Object.keys(json).map((key) => (
            <li>
              <b>"{key}"</b>:{renderJson(json[key])}
            </li>
          ))}
        </ul>
        {"}"}
      </>
    );
  } else {
    if (typeof json === "string") return <span className="string">"{json}"</span>;
    return <span className="other">{json}</span>;
  }
};

export interface Duration {
  secs: number;
  nanos: number;
}

export function displayDuration(duration: Duration) {
  if (duration.secs === 0) {
    if (duration.nanos > 1000000) {
      return `${(duration.nanos / 1000000).toFixed(2)}ms`;
    }
    if (duration.nanos > 1000) {
      return `${(duration.nanos / 1000).toFixed(2)}µs`;
    }
    return `${duration.nanos}ns`;
  }
  if (duration.secs >= 60 * 60) {
    const minutes_secs = duration.secs - Math.floor(duration.secs / 60 / 60) * 60 * 60;
    return `${(duration.secs / 60 / 60).toFixed(0)}h${Math.floor(minutes_secs / 60)}m`;
  }
  if (duration.secs >= 60) {
    const secs = duration.secs - Math.floor(duration.secs / 60) * 60;
    return `${(duration.secs / 60).toFixed(0)}m${secs.toFixed(0)}s`;
  }
  return `${duration.secs}s`;
}


export { renderJson };


export function truncateText(text: string, length: number) {
  if (!length || !text || text.length <= length) {
      return text;
  }
  if (text.length <= length) {
      return text;
  }
  const leftChars = Math.ceil(length/2);
  const rightChars = Math.floor(length/2);
  return text.substring(0, leftChars) + '...' + text.substring(text.length - rightChars);
}
