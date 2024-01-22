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

class U256 {
  n: string;
  constructor(n: string) {
    if (!n) {
      throw new Error("U256 input is null/empty");
    }
    if (n.length > 64) {
      throw new Error("U256 input is larger than 64 characters");
    }
    this.n = n.padStart(64, "0");
  }

  inc() {
    return this.plus(new U256("1"));
  }

  dec() {
    return this.minus(new U256("1"));
  }

  minus(other: U256) {
    let c = 0;
    let s = "";
    for (let i = 63; i >= 0; --i) {
      let t = this.n[i];
      let o = other.n[i];
      let r;
      r = parseInt(t, 16) - (parseInt(o, 16) + c);
      if (r < 0) {
        c = 1;
        r += 16;
      } else {
        c = 0;
      }
      s = r.toString(16) + s;
    }
    return new U256(s);
  }

  plus(other: U256) {
    let c = 0;
    let s = "";
    for (let i = 63; i >= 0; --i) {
      let t = this.n[i];
      let o = other.n[i];
      let r;
      r = parseInt(t, 16) + parseInt(o, 16) + c;
      if (r > 15) {
        c = 1;
        r -= 16;
      } else {
        c = 0;
      }
      s = r.toString(16) + s;
    }
    return new U256(s);
  }

  compare(other: U256) {
    let len = Math.max(this.n.length, other.n.length);
    for (let i = len - 1; i >= 0; --i) {
      let t = this.n?.[this.n.length - 1 - i] || "0";
      let o = other.n?.[other.n.length - 1 - i] || "0";
      if (t > o) return 1;
      if (t < o) return -1;
    }
    return 0;
  }

  gt = (other: U256) => this.compare(other) === 1;
  ge = (other: U256) => this.compare(other) >= 0;
  lt = (other: U256) => this.compare(other) === -1;
  le = (other: U256) => this.compare(other) <= 0;
  eq = (other: U256) => this.compare(other) === 0;
}

function compare(a: any, b: any) {
  if (a < b) return -1;
  if (a > b) return 1;
  return 0;
}

function toHexString(byteArray: number[]) {
  return Array.from(byteArray, function (byte) {
    return ("0" + (byte & 0xff).toString(16)).slice(-2);
  }).join("");
}

function fromHexString(hexString: string) {
  let res = [];
  for (let i = 0; i < hexString.length; i += 2) {
    res.push(Number("0x" + hexString.substring(i, i + 2)));
  }
  return res;
}

function shortenString(string: string, start: number = 8, end: number = 8) {
  return string.substring(0, start) + "..." + string.slice(-end);
}

export { U256, compare, toHexString, fromHexString, shortenString };
