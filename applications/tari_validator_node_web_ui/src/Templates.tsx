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
import { ITemplate } from "./interfaces";
import { getTemplate, getTemplates } from "./json_rpc";
import "./Templates.css";

function Templates() {
  const [templates, setTemplates] = useState([]);
  const [info, setInfo] = useState<{ [id: string]: ITemplate }>();
  const [loading, setLoading] = useState<{ [id: string]: Boolean }>();
  useEffect(() => {
    getTemplates(10).then((response) => {
      setTemplates(response.templates);
    });
  }, []);
  const load = (address: string) => {
    if (info?.[address] || loading?.[address]) {
      return;
    }
    setLoading({ ...loading, [address]: true });
    getTemplate(address).then((response) => {
      setInfo({ ...info, [address]: response });
    });
  };
  const renderFunctions = (template: ITemplate) => {
    return (
      <div>
        <div className="caption">{template.abi.template_name}</div>
        <table>
          <thead>
            <th>Function</th>
            <th>Args</th>
            <th>Returns</th>
          </thead>
          <tbody>
            {template.abi.functions.map((fn) => (
              <tr>
                <td style={{ textAlign: "left" }}>{fn.name}</td>
                <td>{fn.arguments.join(", ")}</td>
                <td>{fn.output}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    );
  };
  return (
    <div className="section">
      <div className="caption">Templates</div>
      <table className="templates">
        <thead>
          <tr>
            <th>Address</th>
            <th>Download URL</th>
            <th>Mined Height</th>
            <th>Status</th>
          </tr>
        </thead>
        <tbody>
          {templates.map(({ address, binary_sha, height, url }) => (
            <tr key={address}>
              <td onMouseOver={() => load(address)} className="tooltip">
                <span className="key">{address}</span>
                {info?.[address] !== undefined ? (
                  <span className="tooltiptext">{renderFunctions(info[address])}</span>
                ) : (
                  <></>
                )}
              </td>
              <td>
                <a href={url}>{url}</a>
              </td>
              <td>{height}</td>
              <td>Active</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export default Templates;
