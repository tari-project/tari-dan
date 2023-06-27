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

const renderJson = (json: any) => {
  if (Array.isArray(json)) {
    if (json.length == 32) {
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
  } else if (typeof json === 'object') {
    return (
      <>
        {'{'}
        <ul>
          {Object.keys(json).map((key) => (
            <li>
              <b>"{key}"</b>:{renderJson(json[key])}
            </li>
          ))}
        </ul>
        {'}'}
      </>
    );
  } else {
    if (typeof json === 'string')
      return <span className="string">"{json}"</span>;
    return <span className="other">{json}</span>;
  }
};


function removeTagged(obj: any) {
  if (obj === undefined) {
    return "undefined";
  }
  if (obj["@@TAGGED@@"] !== undefined) {
    return obj["@@TAGGED@@"][1];
  }
  return obj;
}

function toHexString(byteArray: any ) : string {

  if (Array.isArray( byteArray )) {
    return Array.from(byteArray, function (byte) {
      return ('0' + (byte & 0xff).toString(16)).slice(-2);
    }).join('');
  }
  if (byteArray === undefined) {
    return "undefined";
  }
  // object might be a tagged object
  if (byteArray["@@TAGGED@@"] !== undefined) {
    return toHexString(byteArray["@@TAGGED@@"][1]);
  }
  return "Unsupported type";
}

function fromHexString(hexString: string) {
  let res = [];
  for (let i = 0; i < hexString.length; i += 2) {
    res.push(Number('0x' + hexString.substring(i, i + 2)));
  }
  return res;
}

function shortenString(string: string, start: number = 8, end: number = 8) {
  return string.substring(0, start) + '...' + string.slice(-end);
}

export {renderJson, toHexString, fromHexString, shortenString, removeTagged};
