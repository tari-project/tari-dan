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

import { useEffect, useState } from "react";
import { getNonFungibles, getNonFungibleCount, getNonFungibleCollections } from "../../../utils/json_rpc";
import { Form, useParams } from "react-router-dom";
import { renderJson } from "../../../utils/helpers";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import { DataTableCell, CodeBlock, AccordionIconButton, BoxHeading2 } from "../../../Components/StyledComponents";
import KeyboardArrowDownIcon from "@mui/icons-material/KeyboardArrowDown";
import KeyboardArrowUpIcon from "@mui/icons-material/KeyboardArrowUp";
import Collapse from "@mui/material/Collapse";
import TablePagination from "@mui/material/TablePagination";
import Typography from "@mui/material/Typography";
import { Button, ImageList, ImageListItem, ImageListItemBar, TextField } from "@mui/material";
import AddIcon from "@mui/icons-material/Add";
import { ConfirmDialog } from "../../../Components/AlertDialog";

interface IImageData {
  img: string;
  title: string;
  index: number;
}

function NftGallery() {
  const [items, setItems] = useState<IImageData[]>([]);

  let { resourceAddress } = useParams();

  const updateCollection = () => {
    if (resourceAddress !== undefined) {
      getNonFungibles(resourceAddress, 0, 10).then((resp) => {
        console.log({ resp });

        let nfts: any = [];
        resp.forEach((nft: any, i: number) => {
          console.log(nft);
          let nft_data = nft.substate.substate.NonFungible.data["@@TAGGED@@"][1];
          let { image_url, name } = nft_data;
          console.log(image_url);
          console.log(name);
          nfts.push({ image_url, name, index: i });
        });

        setItems(
          nfts.map((nft: any) => ({
            img: nft.image_url,
            title: nft.name,
            index: nft.index,
          })),
        );
      });
    }
  };

  useEffect(() => {
    updateCollection();
  }, []);

  return (
    <ImageList cols={4} gap={8}>
      {items.map((item) => (
        <ImageListItem key={item.img}>
          <img
            src={`${item.img}?size=248&fit=fill&auto=format`}
            srcSet={`${item.img}?size=248&fit=fill&auto=format&dpr=2 4x`}
            alt={item.title}
            loading="lazy"
          />
          <ImageListItemBar title={item.title} subtitle={<span># {item.index}</span>} position="below" />
        </ImageListItem>
      ))}
    </ImageList>
  );
}

export default NftGallery;
