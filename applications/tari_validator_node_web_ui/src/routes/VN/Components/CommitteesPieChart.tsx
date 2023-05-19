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

const MyChartComponent = ({ chartData }: any) => {
  const [data, setData] = useState([]);
  const [titles, setTitles] = useState([]);

  useEffect(() => {
    const mappedContent = chartData.map(([begin, end, committee]: any) => {
      const data: any = {
        value: committee.length,
        name: `${begin.slice(0, 6)}... - ${end.slice(0, 6)}...`,
        committee: committee,
        link: `/committees/${begin},${end}`,
        range: `${begin}<br />${end}`,
      };
      return data;
    });
    const mappedTitles = chartData.map(([begin, end, committee]: any) => {
      return `${begin.slice(0, 6)}... - ${end.slice(0, 6)}...`;
    });
    setTitles(mappedTitles);
    setData(mappedContent);
  }, [chartData]);

  console.log(titles);

  const tooltipFomatter = (params: any) => {
    const { value, committee, link, range } = params.data;
    return `<b>Range:</b><br />${range}<br />
    <b>${value} Members:</b><br /><ul>
    ${committee
      .map((item: any) => `<li>${item}</li>`)
      .slice(0, 5)
      .join(' ')}</ul><br />
      <a href="${link}">View All Members</a><br />`;
  };

  const option = {
    tooltip: {
      trigger: 'item',
      position: 'right',
      confine: true,
      formatter: tooltipFomatter,
      enterable: true,
    },
    legend: {
      type: 'scroll',
      orient: 'vertical',
      right: 10,
      top: 120,
      bottom: 20,
      data: titles,
    },
    series: [
      {
        type: 'pie',
        radius: '55%',
        center: ['40%', '50%'],
        selectedMode: 'single',
        data: data,
        emphasis: {
          itemStyle: {
            shadowBlur: 10,
            shadowOffsetX: 0,
            shadowColor: 'rgba(0, 0, 0, 0.5)',
          },
        },
      },
    ],
  };

  return <EChartsReact option={option} style={{ height: '600px' }} />;
};

export default MyChartComponent;
