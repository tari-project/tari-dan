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

import { Chip, Avatar } from "@mui/material";
import { IoCheckmarkOutline, IoHourglassOutline, IoCloseOutline, IoBandageOutline } from "react-icons/io5";
import { Decision } from "@tari-project/typescript-bindings";

interface StatusChipProps {
  status: Decision | "Loading" | "Dummy";
  showTitle?: boolean;
}

const colorList: Record<string, string> = {
  Commit: "#5F9C91",
  Loading: "#ECA86A",
  Abort: "#DB7E7E",
  Dummy: "#C0C0C0",
};

const iconList: Record<string, JSX.Element> = {
  Commit: <IoCheckmarkOutline style={{ height: 14, width: 14 }} color="#FFF" />,
  Loading: <IoHourglassOutline style={{ height: 14, width: 14 }} color="#FFF" />,
  Abort: <IoCloseOutline style={{ height: 14, width: 14 }} color="#FFF" />,
  Dummy: <IoBandageOutline style={{ height: 14, width: 14 }} color="#FFF" />,
};

export default function StatusChip({ status, showTitle = true }: StatusChipProps) {
  if (!showTitle) {
    return <Avatar sx={{ bgcolor: colorList[status], height: 22, width: 22 }}>{iconList[status]}</Avatar>;
  } else {
    return (
      <Chip
        avatar={<Avatar sx={{ bgcolor: colorList[status] }}>{iconList[status]}</Avatar>}
        label={status}
        style={{ color: colorList[status], borderColor: colorList[status] }}
        variant="outlined"
      />
    );
  }
}
