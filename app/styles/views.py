from fastapi import APIRouter, Depends
from typing import Any
from app.auth import require_admin, User
from app.styles.services import (
    get_count,
    get_one,
    create_one,
    update_one,
    delete_one,
    get_all,
)
from app.styles.models import StyleRead

router = APIRouter()


@router.get("/{style_id}")
async def get_style(
    obj: StyleRead = Depends(get_one),
    user: User = Depends(require_admin),
) -> StyleRead:
    """Get a style by id"""

    return obj


@router.get("")
async def get_all_styles(
    data: Any = Depends(get_all),
    total_count: int = Depends(get_count),
    user: User = Depends(require_admin),
) -> list[StyleRead]:
    """Get all styles"""

    return data


@router.post("", response_model=StyleRead)
async def create_style(
    obj: StyleRead = Depends(create_one),
    user: User = Depends(require_admin),
) -> StyleRead:
    """Create a style"""

    return obj


@router.put("/{style_id}")
async def update_style(
    style: StyleRead = Depends(update_one),
    user: User = Depends(require_admin),
) -> StyleRead:
    """Update a style on geoserver"""

    return style


@router.delete("/{style_id}")
async def delete_style(
    style_id: str = Depends(delete_one),
    user: User = Depends(require_admin),
) -> str:
    """Delete a style"""

    return style_id
