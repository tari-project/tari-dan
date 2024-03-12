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
import { fromHexString } from "../VN/Components/helpers";
import EChartsReact from "echarts-for-react";
import { ICommitteeChart } from "../../utils/interfaces";
import "../../theme/echarts.css";
import type { CommitteeShardInfo, ValidatorNode } from "@tariproject/typescript-bindings/validator-node-client";

export default function CommitteesWaterfall({ committees }: { committees: CommitteeShardInfo[] }) {
  const [chartData, setChartData] = useState<ICommitteeChart>({
    activeleft: [],
    inactiveleft: [],
    activemiddle: [],
    inactiveright: [],
    activeright: [],
  });
  const [titles, setTitles] = useState<string[]>([]);
  const [divHeight, setDivHeight] = useState<number>(0);

  const TOTAL_WIDTH = 256;
  const INACTIVE_COLOR = "rgba(0, 0, 0, 0)";
  const ACTIVE_COLOR = (params: any) => {
    let index = params.dataIndex;
    var colorList = ["#ECA86A", "#DB7E7E", "#7AC1C2", "#318EFA", "#9D5CF9"];
    return colorList[index % colorList.length];
  };

  useEffect(() => {
    const info: ICommitteeChart = {
      activeleft: [],
      inactiveleft: [],
      activemiddle: [],
      inactiveright: [],
      activeright: [],
    };

    committees.forEach((data) => {
      const firstValue = fromHexString(data.substate_address_range.start)[0];
      const secondValue = fromHexString(data.substate_address_range.end)[0];
      switch (true) {
        case firstValue === secondValue:
          info.activeleft.push(0);
          info.inactiveleft.push(firstValue);
          info.activemiddle.push(2);
          info.inactiveright.push(0);
          info.activeright.push(0);
          break;
        case firstValue < secondValue:
          info.activeleft.push(0);
          info.inactiveleft.push(firstValue);
          info.activemiddle.push(secondValue - firstValue);
          info.inactiveright.push(TOTAL_WIDTH - secondValue);
          info.activeright.push(0);
          break;
        case firstValue > secondValue:
          info.activeleft.push(secondValue);
          info.inactiveleft.push(TOTAL_WIDTH - (TOTAL_WIDTH - firstValue) - secondValue);
          info.activemiddle.push(0);
          info.inactiveright.push(0);
          info.activeright.push(TOTAL_WIDTH - firstValue);
          break;
        default:
          break;
      }
    });
    setChartData(info);
    setDivHeight(committees.length * 50);
    const newTitles = committees.map((data) => `Committee ${data.shard}`);
    setTitles(newTitles);
  }, [committees]);

  function tooltipFormatter(params: any) {
    const dataIndex = params[0].dataIndex;
    const data = committees[dataIndex];
    const begin = fromHexString(data.substate_address_range.start)[0];
    const end = fromHexString(data.substate_address_range.end)[0];

    const memberList = data.validators
      .map((member: ValidatorNode) => `<li>${member.address}</li>`)
      .slice(0, 5)
      .join("");

    return (
      <>
        <b>Range:</b> <br />
        {begin},<br />
        {end}
        <br />
        <b>{data.validators.length} Members:</b> <br />
        <ul>{memberList}</ul>
        <a className="tooltip-btn" href={`committees/${begin},${end}`}>
          View Committee
        </a>
      </>
    );
  }

  const option = {
    grid: {
      left: "3%",
      right: "4%",
      bottom: "3%",
      containLabel: true,
    },
    xAxis: {
      type: "value",
      max: 256,
    },
    yAxis: {
      type: "category",
      data: titles,
      z: 10,
      axisPointer: {
        type: "shadow",
        label: {
          show: true,
          formatter: "{value}",
          textStyle: {
            color: "#fff",
            fontSize: 12,
          },
        },
      },
    },
    tooltip: {
      show: true,
      enterable: true,
      trigger: "axis",
      formatter: tooltipFormatter,
      position: function (point: any) {
        const left = point[0] + 10;
        const top = point[1] - 10;
        return [left, top];
      },
      backgroundColor: "#ffffffe6",
    },
    series: [
      {
        name: "ActiveLeft",
        type: "bar",
        stack: "total",
        label: {
          show: false,
        },
        emphasis: {
          focus: "none",
        },
        data: chartData.activeleft,
        itemStyle: {
          color: ACTIVE_COLOR,
        },
      },
      {
        name: "InactiveLeft",
        type: "bar",
        stack: "total",
        label: {
          show: false,
        },
        emphasis: {
          focus: "none",
        },
        data: chartData.inactiveleft,
        itemStyle: {
          color: INACTIVE_COLOR,
        },
      },
      {
        name: "ActiveMiddle",
        type: "bar",
        stack: "total",
        label: {
          show: false,
        },
        emphasis: {
          focus: "none",
        },
        data: chartData.activemiddle,
        itemStyle: {
          color: ACTIVE_COLOR,
        },
      },
      {
        name: "InactiveRight",
        type: "bar",
        stack: "total",
        label: {
          show: false,
        },
        emphasis: {
          focus: "none",
        },
        data: chartData.inactiveright,
        itemStyle: {
          color: INACTIVE_COLOR,
        },
      },
      {
        name: "ActiveRight",
        type: "bar",
        stack: "total",
        label: {
          show: false,
        },
        emphasis: {
          focus: "none",
        },
        data: chartData.activeright,
        itemStyle: {
          color: ACTIVE_COLOR,
        },
      },
    ],
  };

  return <EChartsReact option={option} style={{ height: divHeight }} />;
}
