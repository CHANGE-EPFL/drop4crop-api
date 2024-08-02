from fastapi import (
    Depends,
    APIRouter,
    Request,
    Query,
    BackgroundTasks,
    Header,
    Response,
    HTTPException,
)
from uuid import uuid4
from typing import Any, Annotated
from collections import defaultdict
from app.auth import require_admin, User
from app.db import get_session, AsyncSession
from sqlmodel import select, update
from app.layers.models import Layer
from app.geoserver.services import process_and_upload_geotiff
from app.s3.services import get_s3
from aioboto3 import Session as S3Session
from app.config import config
from uuid import UUID
import datetime

router = APIRouter()

# In-memory storage for file parts
file_storage = defaultdict(dict)


@router.post("")
async def upload_file(
    request: Request,
    upload_length: int = Header(..., alias="Upload-Length"),
    content_type: str = Header(..., alias="Content-Type"),
    *,
    # background_tasks: BackgroundTasks,
    user: User = Depends(require_admin),
    session: AsyncSession = Depends(get_session),
    s3: S3Session = Depends(get_s3),
) -> UUID:

    try:
        # Initialise object in database
        obj = Layer(
            upload_length=upload_length,
            all_parts_received=False,
        )
        session.add(obj)
        await session.commit()
        await session.refresh(obj)

        # Create multipart upload and add the upload id to the object
        response = await s3.create_multipart_upload(
            Bucket=config.S3_BUCKET_ID,
            Key=f"{config.S3_PREFIX}/{str(obj.id)}",
        )

        if "UploadId" not in response:
            raise HTTPException(
                status_code=500,
                detail="Failed to create object in S3",
            )

        print(f"Initiated object {obj.id} with {upload_length} bytes")

        obj.upload_id = response["UploadId"]

        session.add(obj)
        await session.commit()

    except HTTPException as e:
        raise e
    except Exception as e:
        raise HTTPException(
            status_code=500,
            detail=f"Failed to create object in S3 {e}",
        )

    return obj.id


@router.patch("")
async def upload_chunk(
    request: Request,
    patch: str = Query(...),
    upload_offset: int = Header(..., alias="Upload-Offset"),
    upload_length: int = Header(..., alias="Upload-Length"),
    upload_name: str = Header(..., alias="Upload-Name"),
    content_type: str = Header(..., alias="Content-Type"),
    content_length: int = Header(..., alias="Content-Length"),
    *,
    background_tasks: BackgroundTasks,
    user: User = Depends(require_admin),
    session: AsyncSession = Depends(get_session),
    s3: S3Session = Depends(get_s3),
):
    """Handle chunked file upload"""

    # Clean " from patch
    patch = patch.replace('"', "")

    # Get the object from the DB
    query = select(Layer).where(Layer.id == patch)
    res = await session.exec(query)

    layer = res.one_or_none()
    if not layer:
        raise HTTPException(
            status_code=404,
            detail="Object not found",
        )

    # Extract filename into variables. Structure is:
    # {crop}_{watermodel}_{climatemodel}_{scenario}_{variable}_{year}.tif
    try:
        crop, water_model, climate_model, scenario, variable, year = (
            # Remove file extension then split by _
            upload_name.lower()
            .split(".")[0]
            .split("_")
        )
        print(crop, water_model, climate_model, scenario, variable, year)

    except Exception as e:
        print(e)
        raise HTTPException(
            status_code=400,
            detail={
                "message": "Invalid filename, must be in the format "
                "{crop}_{watermodel}_{climatemodel}_{scenario}_"
                "{variable}_{year}.tif",
            },
        )

    # Query DB to check if the layers exist, if they do, avoid upload
    query = select(Layer).where(
        Layer.crop == crop,
        Layer.water_model == water_model,
        Layer.climate_model == climate_model,
        Layer.scenario == scenario,
        Layer.variable == variable,
        Layer.year == int(year),
        Layer.all_parts_received,
    )

    duplicate_layer = await session.exec(query)
    duplicate_layer = duplicate_layer.one_or_none()

    if duplicate_layer:
        raise HTTPException(
            status_code=409,
            detail={
                "message": "Layer already exists",
            },
        )

    final_part = False
    # Get the part number from the offset
    if upload_offset + content_length == upload_length:
        # The last object is probably not the same size as the other parts, so
        # we need to check if the part number is the last part
        last_part = layer.parts[-1]
        part_number = last_part["PartNumber"] + 1
        final_part = True
        print("THIS IS THE FINAL PART")
    else:
        # Calculate the part number from the division of the offset by the part
        # size (content-length) from the upload length
        part_number = (int(upload_offset) // int(content_length)) + 1
    print(
        f"Upload offset: {upload_offset}, part number: {part_number}, content length: {content_length}, upload length: {upload_length}"
    )
    # Get the chunk data
    data = await request.body()

    try:
        print(
            f"Working on part number {part_number}, chunk {upload_offset} "
            f"{int(upload_offset)+int(content_length)} "
            f"of {upload_length} bytes "
            f"({int(upload_offset)/int(upload_length)*100}%)"
        )
        part = await s3.upload_part(
            Bucket=config.S3_BUCKET_ID,
            Key=f"{config.S3_PREFIX}/{str(layer.id)}",
            UploadId=layer.upload_id,
            PartNumber=part_number,
            Body=data,
        )
        if layer.parts is None or not layer.parts:
            layer.parts = []

        layer.parts += [
            {
                "PartNumber": part_number,
                "ETag": part["ETag"],
                "Size": content_length,
                "Offset": upload_offset,
                "Length": content_length,
            }
        ]
        layer.filename = upload_name
        layer.last_part_received_utc = datetime.datetime.now()
        session.add(layer)
        await session.commit()
        await session.refresh(layer)

        if final_part:
            # Complete the multipart upload
            # Simplify the parts list to only include the PartNumber and ETag
            parts_list = [
                {"PartNumber": x["PartNumber"], "ETag": x["ETag"]}
                for x in layer.parts
            ]
            res = await s3.complete_multipart_upload(
                Bucket=config.S3_BUCKET_ID,
                Key=f"{config.S3_PREFIX}/{str(layer.id)}",
                UploadId=layer.upload_id,
                MultipartUpload={"Parts": parts_list},
            )

            if res is None:
                raise HTTPException(
                    status_code=500,
                    detail="Failed to complete upload",
                )

            layer.all_parts_received = True
            layer.crop = crop
            layer.water_model = water_model
            layer.climate_model = climate_model
            layer.scenario = scenario
            layer.variable = variable
            layer.year = int(year)

            layer.processing_completed_successfully = True
            layer.all_parts_received = True

            session.add(layer)
            await session.commit()
            await session.refresh(layer)

    except Exception as e:
        print(e)
        raise HTTPException(
            status_code=500,
            detail=f"Failed to upload file: {e}",
        )

    return {"message": "Chunk uploaded successfully"}


@router.head("")
async def check_uploaded_chunks(
    response: Response,
    patch: str = Query(...),
    user: User = Depends(require_admin),
    session: AsyncSession = Depends(get_session),
):
    """Responds with the offset of the next expected chunk"""
    # Get the InputObject from the DB
    # Clean " from patch
    patch = patch.replace('"', "")
    query = select(Layer).where(Layer.id == patch)

    res = await session.exec(query)
    object = res.one_or_none()

    if not object:
        raise HTTPException(
            status_code=404,
            detail="Object not found",
        )

    # Calculate the next expected offset
    next_expected_offset = 0
    if object.parts:
        last_part = object.parts[-1]
        next_expected_offset = last_part["Offset"] + last_part["Length"]

    # Return headers with Upload-Offset
    response.headers["Upload-Offset"] = str(next_expected_offset)

    return
