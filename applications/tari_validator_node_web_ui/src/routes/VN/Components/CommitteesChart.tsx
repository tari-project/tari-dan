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
import EChartsReact from 'echarts-for-react';
import { fromHexString } from './helpers';

interface IChartProps {
  0: string;
  1: string;
  2: string[];
}

interface IChartProps2 {
  chartData: IChartProps[];
}

interface ICommitteeData {
  name: string;
  value: number;
  tooltip: string;
}

interface IChartData {
  0: string;
  1: number;
  2: number;
  3: string;
  4: ICommitteeData[];
  5: string;
  6: string;
}

const MyChartComponent = ({ chartData }: IChartProps2) => {
  const [chartInfo, setChartInfo] = useState<IChartData[]>([]);

  // The chart needs an array with the following info:
  // [0] Location for the dot horizontally
  // [1] Location vertically - they are currently all on the same line
  // [2] Size of the dot determined by the number of members
  // [3] Tooltip heading
  // [4] Committee info for the tooltip
  // [5] Beginning value
  // [6] Ending value

  useEffect(() => {
    const mappedContent = chartData.map(([begin, end, committee]: any) => {
      const data: IChartData = [
        fromHexString(begin)[0].toString(),
        0,
        committee.length,
        `${begin}, <br /> ${end}`,
        committee.map((data: any) => ({
          name: fromHexString(data)[0].toString(),
          value: 1,
          tooltip: data,
        })),
        begin,
        end,
      ];
      return data;
    });
    setChartInfo(mappedContent);
  }, [chartData]);

  const range = Array.from({ length: 256 }, (_, i) => i + 1);

  // prettier-ignore
  const legend = [
      'Committees'
    ];

  const option = {
    dataZoom: [
      {
        id: 'dataZoomX',
        type: 'slider',
        xAxisIndex: [0],
        filterMode: 'filter',
      },
    ],
    tooltip: {
      position: 'top',
      confine: true,
      enterable: true,
      textStyle: {
        fontSize: 12,
      },
      axisPointer: {
        type: 'cross',
      },
      formatter: function ({ value }: any) {
        return `<b>Range:</b><br />${value[3]}<br /><hr /><b>${
          value[4].length
        } Members:</b><br />${value[4]
          .map((item: any) => `<li>${item.tooltip}</li>`)
          .slice(0, 5)
          .join(' ')}<hr /><a href="/committees/${value[5]},${
          value[6]
        }">View All Members</a><br />
          `;
      },
    },
    toolbox: {
      feature: {
        dataZoom: {
          yAxisIndex: 'none',
        },
      },
    },
    grid: {
      left: 2,
      bottom: 80,
      right: 10,
      containLabel: false,
    },
    xAxis: {
      type: 'category',
      data: range,
      boundaryGap: false,
      splitLine: {
        show: true,
      },
      axisLine: {
        show: false,
      },
    },
    yAxis: {
      type: 'category',
      data: legend,
      axisLine: {
        show: false,
      },
    },
    series: [
      {
        name: 'Committees',
        type: 'scatter',
        // determine the dot size based on number of children
        // the maximum dot size that fits inside the chart is about 150
        symbolSize: function (val: any) {
          switch (true) {
            case val[2] === 0:
              // minimum
              return 10;
            case val[2] > 0 && val[2] <= 30:
              // inbetween
              return val[2] * 5;
            case val[2] > 30:
              // maximum
              return 150;
            default:
              return 10;
          }
        },
        data: chartInfo,
        animationDelay: function (idx: any) {
          return idx * 5;
        },
        colorBy: 'data',
      },
    ],
  };

  return <EChartsReact option={option} />;
};

export default MyChartComponent;
