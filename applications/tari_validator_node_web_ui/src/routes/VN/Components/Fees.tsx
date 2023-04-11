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

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { addPeer, getConnections, getFees } from '../../../utils/json_rpc';
import { toHexString, shortenString } from './helpers';
import Table from '@mui/material/Table';
import TableBody from '@mui/material/TableBody';
import TableCell from '@mui/material/TableCell';
import TableContainer from '@mui/material/TableContainer';
import TableHead from '@mui/material/TableHead';
import TableRow from '@mui/material/TableRow';
import {
    DataTableCell,
    BoxHeading2,
} from '../../../Components/StyledComponents';
import AddIcon from '@mui/icons-material/Add';
import Button from '@mui/material/Button';
import { TextField } from '@mui/material';
import { Form } from 'react-router-dom';
import Fade from '@mui/material/Fade';
import CopyToClipboard from '../../../Components/CopyToClipboard';

interface IFees {
    epoch: number;
    claimablePublicKey: string;
    totalAccruedFee: number;
}

const useInterval = (fn: () => Promise<unknown>, ms: number) => {
    const timeout = useRef<number>();
    const mountedRef = useRef(false);
    const run = useCallback(async () => {
        await fn();
        if (mountedRef.current) {
            timeout.current = window.setTimeout(run, ms);
        }
    }, [fn, ms]);
    useEffect(() => {
        mountedRef.current = true;
        run();
        return () => {
            mountedRef.current = false;
            window.clearTimeout(timeout.current);
        };
    }, [run]);
};

function Fees() {
    const [connections, setConnections] = useState<IFees[]>([]);
    const [showPeerDialog, setShowAddPeerDialog] = useState(false);
    const [formState, setFormState] = useState({ epoch: '', publicKey: '' });

    const showAddPeerDialog = (setElseToggle: boolean = !showPeerDialog) => {
        setShowAddPeerDialog(setElseToggle);
    };

    const getVNFees = () => {
        getFees(formState.epoch, formState.publicKey);
        setShowAddPeerDialog(false);
    };
    const onChange = (e: React.ChangeEvent<HTMLInputElement>) => {
        setFormState({ ...formState, [e.target.name]: e.target.value });
    };

    useInterval(getVNFees, 5000);

    return (
        <>
            <BoxHeading2>
                {showPeerDialog && (
                    <Fade in={showPeerDialog}>
                        <Form onSubmit={getFees} className="flex-container">
                            <TextField
                                name="epoch"
                                label="Epoch"
                                value={formState.epoch}
                                onChange={onChange}
                                style={{ flexGrow: 1 }}
                            />
                            <TextField
                                name="claimablePublicKey"
                                label="VN Public Key"
                                value={formState.publicKey}
                                onChange={onChange}
                                style={{ flexGrow: 1 }}
                            />
                            <Button variant="contained" type="submit">
                                Add Peer
                            </Button>
                            <Button
                                variant="outlined"
                                onClick={() => showAddPeerDialog(false)}
                            >
                                Cancel
                            </Button>
                        </Form>
                    </Fade>
                )}
                {!showPeerDialog && (
                    <Fade in={!showPeerDialog}>
                        <div className="flex-container">
                            <Button
                                variant="outlined"
                                startIcon={<AddIcon />}
                                onClick={() => showAddPeerDialog()}
                            >
                                Add Peer
                            </Button>
                        </div>
                    </Fade>
                )}
            </BoxHeading2>
            <TableContainer>
                <Table>
                    <TableHead>
                        <TableRow>
                            <TableCell>Epoch</TableCell>
                            <TableCell>claimablePublicKey</TableCell>
                            <TableCell>totalAccruedFee</TableCell>
                        </TableRow>
                    </TableHead>
                    <TableBody>
                        {connections.map(
                            ({ epoch, claimablePublicKey, totalAccruedFee }) => (
                                <TableRow key={claimablePublicKey}>
                                    <DataTableCell>{epoch}</DataTableCell>
                                    <DataTableCell>{claimablePublicKey}</DataTableCell>
                                    <DataTableCell>{totalAccruedFee}</DataTableCell>

                                </TableRow>
                            )
                        )}
                    </TableBody>
                </Table>
            </TableContainer>
        </>
    );
}

export default Fees;
