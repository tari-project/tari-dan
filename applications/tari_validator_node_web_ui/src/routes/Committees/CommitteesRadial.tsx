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

import { useState, useEffect } from 'react';
import { fromHexString } from '../VN/Components/helpers';
import EChartsReact from 'echarts-for-react';
import {
  ICommittees,
  ICommitteeChart,
  ICommitteeMap,
} from '../../utils/interfaces';
import '../../theme/echarts.css';

export default function CommitteesRadial({
  committees,
}: {
  committees: ICommittees;
}) {
  const [chartData, setChartData] = useState<ICommitteeChart>({
    activeleft: [],
    inactiveleft: [],
    activemiddle: [],
    inactiveright: [],
    activeright: [],
  });
  const [titles, setTitles] = useState<string[]>([]);

  const TOTAL_WIDTH = 256;
  const INACTIVE_COLOR = 'rgba(0, 0, 0, 0)';
  const ACTIVE_COLOR = (params: any) => {
    let index = params.dataIndex;
    var colorList = ['#ECA86A', '#DB7E7E', '#7AC1C2', '#318EFA', '#9D5CF9'];
    return colorList[index % colorList.length];
  };

  useEffect(() => {
    const dataset = committees.map(
      ([begin, end, committee]: ICommittees[number]) => {
        const data: ICommitteeMap = [
          fromHexString(begin)[0],
          fromHexString(end)[0],
          committee,
        ];
        return data;
      }
    );

    const info: ICommitteeChart = {
      activeleft: [],
      inactiveleft: [],
      activemiddle: [],
      inactiveright: [],
      activeright: [],
    };

    dataset.forEach((data: ICommitteeMap) => {
      const [firstValue, secondValue] = data;
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
          info.inactiveleft.push(data[0]);
          info.activemiddle.push(data[1] - data[0]);
          info.inactiveright.push(TOTAL_WIDTH - data[1]);
          info.activeright.push(0);
          break;
        case firstValue > secondValue:
          info.activeleft.push(data[1]);
          info.inactiveleft.push(
            TOTAL_WIDTH - (TOTAL_WIDTH - data[0]) - data[1]
          );
          info.activemiddle.push(0);
          info.inactiveright.push(0);
          info.activeright.push(TOTAL_WIDTH - data[0]);
          break;
        default:
          break;
      }
    });
    setChartData(info);
    const newTitles = dataset.map(
      (data: ICommitteeMap, index: number) => `Committee ${index + 1}`
    );
    setTitles(newTitles);
  }, [committees]);

  function tooltipFormatter(params: any) {
    const dataIndex = params[0].dataIndex;
    const data = committees[dataIndex];
    const members = data[2] as string[];
    const begin = data[0] as string;
    const end = data[1] as string;

    const memberList = members
      .map((member: string) => `<li>${member}</li>`)
      .slice(0, 5)
      .join('');

    return `<b>Range:</b> <br />
            ${begin},<br />
            ${end}<br />
            <b>${members.length} Members:</b> <br />
            <ul>${memberList}</ul>
            <a class="tooltip-btn" href="committees/${begin},${end}">View Committee</a>
            `;
  }

  const option = {
    angleAxis: {
      max: TOTAL_WIDTH,
    },
    radiusAxis: {
      type: 'category',
      data: titles,
      z: 10,
      axisPointer: {
        type: 'shadow',
        label: {
          show: true,
          formatter: '{value}',
          textStyle: {
            color: '#fff',
            fontSize: 12,
          },
        },
      },
    },
    tooltip: {
      show: true,
      enterable: true,
      trigger: 'axis',
      formatter: tooltipFormatter,
      position: function (point: any) {
        const left = point[0] + 10;
        const top = point[1] - 10;
        return [left, top];
      },
      backgroundColor: '#ffffffe6',
    },
    polar: {},
    series: [
      {
        type: 'bar',
        data: chartData.activeleft,
        coordinateSystem: 'polar',
        name: 'ActiveLeft',
        stack: 'a',
        emphasis: {
          focus: 'none',
        },
        itemStyle: {
          color: ACTIVE_COLOR,
        },
      },
      {
        type: 'bar',
        data: chartData.inactiveleft,
        coordinateSystem: 'polar',
        name: 'InactiveLeft',
        stack: 'a',
        emphasis: {
          focus: 'none',
        },
        itemStyle: {
          color: INACTIVE_COLOR,
        },
      },
      {
        type: 'bar',
        data: chartData.activemiddle,
        coordinateSystem: 'polar',
        name: 'ActiveMiddle',
        stack: 'a',
        emphasis: {
          focus: 'none',
        },
        itemStyle: {
          color: ACTIVE_COLOR,
        },
      },
      {
        type: 'bar',
        data: chartData.inactiveright,
        coordinateSystem: 'polar',
        name: 'InactiveRight',
        stack: 'a',
        emphasis: {
          focus: 'none',
        },
        itemStyle: {
          color: INACTIVE_COLOR,
        },
      },
      {
        type: 'bar',
        data: chartData.activeright,
        coordinateSystem: 'polar',
        name: 'ActiveRight',
        stack: 'a',
        emphasis: {
          focus: 'none',
        },
        itemStyle: {
          color: ACTIVE_COLOR,
        },
      },
    ],
  };

  return <EChartsReact option={option} style={{ height: 600 }} />;
}
