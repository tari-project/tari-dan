//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

interface NodeControlsProps {
  isRunning: boolean,
  onStart: () => void;
  onStop: () => void;
  onDeleteData: () => void;
}

export default function NodeControls({ isRunning, onStart, onStop, onDeleteData }: NodeControlsProps) {
  return <>
    <button onClick={onStart} disabled={isRunning}>Start</button>
    <button onClick={onStop} disabled={!isRunning}>Stop</button>
    <button onClick={onDeleteData}>Delete data</button>
  </>;
}

