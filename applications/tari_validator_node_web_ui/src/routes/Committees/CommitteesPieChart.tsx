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

import { useState, useEffect } from "react";
import EChartsReact from "echarts-for-react";
import "../../theme/echarts.css";
import type {
  CommitteeShardInfo,
  GetNetworkCommitteeResponse,
} from "@tariproject/typescript-bindings/validator-node-client";

interface IData {
  value: number;
  name: string;
  committee: any;
  link: string;
  range: string;
}

const MyChartComponent = ({ chartData }: MyChartComponentProps) => {
  const [data, setData] = useState<IData[]>([]);
  const [titles, setTitles] = useState<string[]>([]);

  useEffect(() => {
    const mappedTitles = chartData.committees.map((shardInfo: CommitteeShardInfo) => {
      return `${shardInfo.substate_address_range.start.slice(0, 6)}... - ${shardInfo.substate_address_range.end.slice(
        0,
        6,
      )}...`;
    });
    setTitles(mappedTitles);

    const mappedContent = chartData.committees.reverse().map((shardInfo) => ({
      value: shardInfo.validators.length,
      name: `${shardInfo.substate_address_range.start.slice(0, 6)}... - ${shardInfo.substate_address_range.end.slice(
        0,
        6,
      )}...`,
      committee: shardInfo.validators,
      link: `/committees/${shardInfo.substate_address_range.start},${shardInfo.substate_address_range.end}`,
      range: `${shardInfo.substate_address_range.start}<br />${shardInfo.substate_address_range.end}`,
    }));
    setData(mappedContent);
  }, [chartData]);

  console.log(titles);

  const tooltipFormatter = (params: any) => {
    const { committee, link, range } = params.data;
    return `<b>Range:</b><br />${range}<br />
    <b>Bucket:</b> ${committee[0].committee_bucket} <br/>
    <b>Members:</b> ${committee.length}<br /><ul>
    ${committee
      .map((item: any) => `<li>Address: ${item.address}</li>`)
      .slice(0, 5)
      .join(" ")}</ul><br />
      <a class="tooltip-btn" href="${link}">View All Members</a><br />`;
  };

  const option = {
    tooltip: {
      trigger: "item",
      position: "right",
      confine: true,
      formatter: tooltipFormatter,
      enterable: true,
      backgroundColor: "#ffffffe6",
    },
    legend: {
      type: "scroll",
      orient: "vertical",
      right: 10,
      top: 120,
      bottom: 20,
      data: titles,
    },
    series: [
      {
        type: "pie",
        radius: "55%",
        center: ["40%", "50%"],
        selectedMode: "single",
        data: data,
        emphasis: {
          itemStyle: {
            shadowBlur: 10,
            shadowOffsetX: 0,
            shadowColor: "rgba(0, 0, 0, 0.5)",
          },
        },
      },
    ],
  };

  return <EChartsReact option={option} style={{ height: "600px" }} />;
};

interface MyChartComponentProps {
  chartData: GetNetworkCommitteeResponse;
}

export default MyChartComponent;
