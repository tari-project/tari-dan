import { useState, useEffect } from 'react';
import { fromHexString } from './helpers';
import EChartsReact from 'echarts-for-react';
import theme from '../../../theme/theme';

type CommitteeData = [string, string, string[]][];

export default function CommitteesWaterfall({
  committees,
}: {
  committees: CommitteeData;
}) {
  const [chartData, setChartData] = useState<any>({
    activeleft: [],
    inactiveleft: [],
    activemiddle: [],
    inactiveright: [],
    activeright: [],
  });
  const [titles, setTitles] = useState<any[]>([]);
  const [divHeight, setDivHeight] = useState<number>(0);

  const TOTAL_WIDTH = 256;
  const ACTIVE_COLOR = theme.palette.primary.dark;
  const INACTIVE_COLOR = 'rgba(0, 0, 0, 0)';

  useEffect(() => {
    const info: any = {
      activeleft: [],
      inactiveleft: [],
      activemiddle: [],
      inactiveright: [],
      activeright: [],
    };

    const dataset = committees.map(([begin, end, committee]: any) => {
      const data: any = [
        fromHexString(begin)[0],
        fromHexString(end)[0],
        committee,
      ];
      return data;
    });

    dataset.forEach((data: any) => {
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
    setDivHeight(dataset.length * 40);
    const newTitles = dataset.map(
      (data: any, index: any) => `Committee ${index + 1}`
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
    grid: {
      left: '3%',
      right: '4%',
      bottom: '3%',
      containLabel: true,
    },
    xAxis: {
      type: 'value',
      max: 256,
    },
    yAxis: {
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
    series: [
      {
        name: 'ActiveLeft',
        type: 'bar',
        stack: 'total',
        label: {
          show: false,
        },
        emphasis: {
          focus: 'none',
        },
        data: chartData.activeleft,
        itemStyle: {
          color: ACTIVE_COLOR,
        },
      },
      {
        name: 'InactiveLeft',
        type: 'bar',
        stack: 'total',
        label: {
          show: false,
        },
        emphasis: {
          focus: 'none',
        },
        data: chartData.inactiveleft,
        itemStyle: {
          color: INACTIVE_COLOR,
        },
      },
      {
        name: 'ActiveMiddle',
        type: 'bar',
        stack: 'total',
        label: {
          show: false,
        },
        emphasis: {
          focus: 'none',
        },
        data: chartData.activemiddle,
        itemStyle: {
          color: ACTIVE_COLOR,
        },
      },
      {
        name: 'InactiveRight',
        type: 'bar',
        stack: 'total',
        label: {
          show: false,
        },
        emphasis: {
          focus: 'none',
        },
        data: chartData.inactiveright,
        itemStyle: {
          color: INACTIVE_COLOR,
        },
      },
      {
        name: 'ActiveRight',
        type: 'bar',
        stack: 'total',
        label: {
          show: false,
        },
        emphasis: {
          focus: 'none',
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
