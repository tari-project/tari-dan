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
import { getNonFungibles } from "../../../utils/json_rpc";
import { useParams } from "react-router-dom";
import { ImageList, ImageListItem, ImageListItemBar } from "@mui/material";
import type { NonFungibleSubstate } from "@tariproject/typescript-bindings/tari-indexer-client";

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
      getNonFungibles({ address: { Resource: resourceAddress }, start_index: 0, end_index: 10 }).then((resp) => {
        let nfts: any = [];
        resp.non_fungibles.forEach((nft: NonFungibleSubstate, i: number) => {
          console.log(nft);
          if (!("NonFungible" in nft.substate.substate)) {
            return;
          }
          // let nft_data = nft.substate.substate.NonFungible?.data;
          // Was this is a work in progress? There was no image_url coming from the jrpc before this change.
          // TODO: make this work
          // let { image_url, name } = nft_data;
          // console.log(image_url);
          // console.log(name);
          // nfts.push({ image_url, name, index: i });
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
  });

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
