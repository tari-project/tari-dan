// BigInt holds the value as a hex string
class U256 {
  n: string;
  constructor(n: string) {
    if (n.length > 64) {
      throw new Error("Input is bigger than it should");
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

export { U256, compare };
