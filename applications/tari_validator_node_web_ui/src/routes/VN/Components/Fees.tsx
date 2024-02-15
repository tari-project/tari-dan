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

import { useContext, useEffect, useState } from "react";
import { getFees } from "../../../utils/json_rpc";
import { VNContext } from "../../../App";
import EChartsReact from "echarts-for-react";

function Fees() {
  const [totalFeesPerEpoch, setTotalFeesPerEpoch] = useState<number[]>([]);
  const [dueFeesPerEpoch, setDueFeesPerEpoch] = useState<number[]>([]);
  const [minEpoch, setMinEpoch] = useState(0);
  const [maxEpoch, setMaxEpoch] = useState(0);

  const { epoch, identity } = useContext(VNContext);

  useEffect(() => {
    if (epoch !== undefined && identity !== undefined) {
      // console.log(identity)
      getFees({ epoch_range: { start: 0, end: epoch.current_epoch }, validator_public_key: identity.public_key }).then(
        (resp) => {
          let min_epoch = epoch.current_epoch;
          let max_epoch = 0;
          let total_fees: { [epoch: number]: number } = {};
          let fees_due: { [epoch: number]: number } = {};
          for (let fees of resp.fees) {
            min_epoch = Math.min(min_epoch, fees.epoch);
            max_epoch = Math.max(max_epoch, fees.epoch);
            if (!(fees.epoch in total_fees)) {
              total_fees[fees.epoch] = fees.total_transaction_fee;
              fees_due[fees.epoch] = fees.total_fee_due;
            } else {
              total_fees[fees.epoch] += fees.total_transaction_fee;
              fees_due[fees.epoch] += fees.total_fee_due;
            }
          }
          setMinEpoch(min_epoch);
          setMaxEpoch(max_epoch);
          setTotalFeesPerEpoch(
            Array.from({ length: max_epoch - min_epoch + 1 }, (_, i) => total_fees[i + min_epoch] || 0),
          );
          setDueFeesPerEpoch(Array.from({ length: max_epoch - min_epoch + 1 }, (_, i) => fees_due[i + min_epoch] || 0));
        },
      );
    }
  }, [identity, epoch]);

  if (epoch === undefined || identity === undefined) return <div>Loading</div>;

  const options = {
    title: {
      text: "Fees per epoch",
    },
    legend: {
      data: ["Total fees", "Due fees"],
    },
    xAxis: {
      type: "category",
      boundaryGap: false,
      data: Array.from({ length: maxEpoch - minEpoch + 1 }, (_, i) => i + minEpoch),
    },
    yAxis: {
      type: "value",
    },
    series: [
      {
        name: "Total fees",
        type: "line",
        data: totalFeesPerEpoch,
        areaStyle: {},
        label: {
          show: true,
          position: "top",
        },
      },
      {
        name: "Due fees",
        type: "line",
        data: dueFeesPerEpoch,
        areaStyle: {},
        label: {
          show: true,
          position: "top",
        },
      },
    ],
  };

  return (
    <>
      <EChartsReact option={options} style={{ height: "600px" }} />
    </>
  );
}

export default Fees;
